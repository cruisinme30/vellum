// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The Electron main process: wires Electron to the tested modules.

import { hostname } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  BrowserWindow,
  Notification,
  app,
  ipcMain,
  net,
  protocol,
  safeStorage,
  session,
  shell,
} from "electron";
import electronUpdater from "electron-updater";

import { Channel } from "../shared/channels.js";

import { handleAppRequest } from "./app-protocol.js";
import { Session } from "./auth.js";
import { DEEP_LINK_SCHEME, deepLinkPath, findDeepLink } from "./deep-link.js";
import {
  fileDisk,
  freePort,
  readOptionalFile,
  runBinary,
  safeStorageVault,
  spawnChild,
} from "./electron-adapters.js";
import type { Fetch } from "./fetch.js";
import { bridgeHandlers, dispatch } from "./ipc.js";
import { PrivateSpaces } from "./private.js";
import {
  APP_HOST,
  APP_SCHEME,
  APP_URL,
  allowPermission,
  contentSecurityPolicy,
  isAppUrl,
  isSafeExternal,
  webPreferences,
} from "./security.js";
import { loadSettings, saveSettings, settingsFromArgv } from "./settings.js";
import { startSidecar, type Sidecar } from "./sidecar.js";
import { TokenStore } from "./token-store.js";
import { startUpdates } from "./updater.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const resources = app.isPackaged ? process.resourcesPath : path.resolve(here, "../../..");
const webRoot = app.isPackaged ? path.join(resources, "web") : path.join(resources, "web", "dist");
const binaryName = process.platform === "win32" ? "vellum.exe" : "vellum";
const binary = app.isPackaged
  ? path.join(resources, "bin", binaryName)
  : path.join(resources, "target", "debug", binaryName);
const fetchFromMain: Fetch = (input, init) => net.fetch(input, init);

protocol.registerSchemesAsPrivileged([
  {
    scheme: APP_SCHEME,
    privileges: { standard: true, secure: true, supportFetchAPI: true, codeCache: true },
  },
]);

if (!app.requestSingleInstanceLock()) app.quit();
app.setAsDefaultProtocolClient(DEEP_LINK_SCHEME);

let window: BrowserWindow | undefined;
let sidecar: Sidecar | undefined;
let pendingLink = findDeepLink(process.argv);

function openLink(link: string): void {
  const target = deepLinkPath(link);
  if (target === undefined) return;
  if (window === undefined) {
    pendingLink = target;
    return;
  }
  if (window.isMinimized()) window.restore();
  window.focus();
  window.webContents.send(Channel.openUrl, target);
}

app.on("second-instance", (_event, argv) => {
  const link = argv.find((argument) => argument.startsWith(`${DEEP_LINK_SCHEME}://`));
  if (link === undefined) window?.focus();
  else openLink(link);
});
app.on("open-url", (event, link) => {
  event.preventDefault();
  openLink(link);
});

// Every web contents: no new windows, no navigation away, links go to the system browser.
app.on("web-contents-created", (_event, contents) => {
  contents.setWindowOpenHandler(({ url }) => {
    if (isSafeExternal(url)) void shell.openExternal(url);
    return { action: "deny" };
  });
  contents.on("will-navigate", (event, url) => {
    if (!isAppUrl(url)) event.preventDefault();
  });
  contents.on("will-attach-webview", (event) => {
    event.preventDefault();
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});
app.on("before-quit", () => sidecar?.stop());

async function serverUrl(): Promise<string> {
  const settingsDisk = fileDisk(path.join(app.getPath("userData"), "settings.json"));
  const fromArgv = settingsFromArgv(process.argv);
  if (fromArgv !== undefined) await saveSettings(settingsDisk, fromArgv);
  const settings = fromArgv ?? (await loadSettings(settingsDisk));
  if (settings.mode === "remote") return settings.serverUrl;
  sidecar = await startSidecar(
    { binary, dataDir: path.join(app.getPath("userData"), "workspace") },
    {
      spawn: spawnChild,
      freePort,
      fetch: fetchFromMain,
      sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
      now: Date.now,
    },
  );
  return sidecar.url;
}

function createWindow(): BrowserWindow {
  const created = new BrowserWindow({
    width: 1280,
    height: 832,
    minWidth: 360,
    minHeight: 480,
    show: false,
    title: "Vellum",
    backgroundColor: "#ffffff",
    titleBarStyle: process.platform === "darwin" ? "hiddenInset" : "default",
    webPreferences: webPreferences(path.join(here, "../preload/preload.cjs"), app.getVersion()),
  });
  created.once("ready-to-show", () => {
    created.show();
  });
  created.on("closed", () => {
    window = undefined;
  });
  const start = pendingLink ?? "/";
  pendingLink = undefined;
  void created.loadURL(new URL(start, APP_URL).href);
  return created;
}

async function start(): Promise<void> {
  await app.whenReady();
  const server = await serverUrl();
  const tokens = new TokenStore(
    safeStorageVault(safeStorage),
    fileDisk(path.join(app.getPath("userData"), "session.bin")),
  );
  const auth = new Session(server, tokens, {
    fetch: fetchFromMain,
    openBrowser: (url) => shell.openExternal(url),
    now: Date.now,
  });

  protocol.handle(APP_SCHEME, (request) =>
    handleAppRequest(request, {
      webRoot,
      server: () => server,
      accessToken: () => auth.accessToken(),
      readFile: readOptionalFile,
      fetch: fetchFromMain,
      csp: () => contentSecurityPolicy(server),
    }),
  );
  session.defaultSession.setPermissionRequestHandler((contents, permission, callback) => {
    callback(allowPermission(permission, contents.getURL()));
  });
  session.defaultSession.setPermissionCheckHandler((_contents, permission, origin) =>
    allowPermission(permission, origin === `${APP_SCHEME}://${APP_HOST}` ? APP_URL : origin),
  );

  const privateSpaces = new PrivateSpaces(
    () => server,
    () => auth.accessToken(),
    { run: runBinary(binary), hostname },
  );
  const handlers = bridgeHandlers({
    serverUrl: () => server,
    signIn: () => auth.signIn(),
    signOut: () => auth.signOut(),
    accessToken: () => auth.accessToken(),
    openExternal: (url) => shell.openExternal(url),
    privateDeviceKey: () => privateSpaces.deviceKey(),
    privateEnrol: (spaceId, label) => privateSpaces.enrol(spaceId, label),
    privateReseal: (spaceId) => privateSpaces.reseal(spaceId),
    notify(notice) {
      const shown = new Notification({ title: notice.title, body: notice.body });
      shown.on("click", () => {
        openLink(`${DEEP_LINK_SCHEME}:/${notice.url}`);
      });
      shown.show();
    },
  });
  for (const channel of Object.values(Channel)) {
    if (channel === Channel.openUrl) continue;
    ipcMain.handle(channel, (event, input: unknown) =>
      dispatch(handlers, channel, event.senderFrame?.url ?? "", input),
    );
  }

  window = createWindow();
  app.on("activate", () => {
    window ??= createWindow();
  });

  if (app.isPackaged) {
    // electron-updater is CommonJS and defines autoUpdater lazily, so a named ESM import does not see it.
    // eslint-disable-next-line import-x/no-named-as-default-member
    startUpdates(electronUpdater.autoUpdater, {
      onReady(version) {
        new Notification({
          title: "Vellum update ready",
          body: `Version ${version} installs when you quit.`,
        }).show();
      },
      onError(error) {
        console.warn("update check failed", error.message);
      },
    });
  }
}

// Not top-level await: Electron emits "ready" only after the entry module
// has finished evaluating, so awaiting `whenReady()` here would deadlock.
// eslint-disable-next-line unicorn/prefer-top-level-await
start().catch((error: unknown) => {
  console.error(error);
  app.exit(1);
});
