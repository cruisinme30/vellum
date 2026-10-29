---
id: 01M63CFE61MH3QDCDGEYXK3X8P
title: Encryption and keys
vellum: 1
---

# Encryption and keys

## Envelope encryption {#envelope}

Every document is encrypted with its own data key (XChaCha20-Poly1305).
Each data key is encrypted, or *wrapped*, with the workspace key, which
never touches disk unprotected. Rotating the workspace key re-wraps the
data keys in the background without re-encrypting any content.

```mermaid
flowchart LR
  P[Page] -->|encrypted with| DEK[Data key]
  DEK -->|wrapped by| KEK[Workspace key]
  KEK -->|held by| H[Keychain, KMS, Vault or passphrase]
```

## Where the workspace key lives {#workspace-key}

| Option | Fits |
| --- | --- |
| Operating system keychain | The single binary on a laptop or desktop |
| AWS KMS, Google Cloud KMS, HashiCorp Vault | Servers and clusters |
| Passphrase (Argon2id) | Servers without a key service; you type it at start |

Choose in **Admin → Security → Keys** or with `vellum keys init`.

## The index {#index}

The local SQLite index is encrypted with SQLCipher. In the team and org
profiles, use Postgres with transparent data encryption or encrypted
disks.

## Plaintext spaces {#plaintext}

A space can store plain Markdown on purpose, for example a docs space
that lives in a public Git repository. Turning on plaintext is an
explicit per-space setting, recorded in the audit log, and the space
shows a badge.

## End-to-end encrypted spaces {#e2e}

In an end-to-end encrypted space, members' devices encrypt every page
to all members' keys (X25519). The server stores only ciphertext, so
server-side search, previews, public links and network MCP are not
available there; the desktop app provides search and MCP locally.

::: crit
If every member of an end-to-end encrypted space loses their device
keys, nobody, including you, can recover its pages. Ask members to
export a recovery key when they join.
:::

## Rotation {#rotation}

```bash
vellum keys rotate          # new workspace key; data keys are re-wrapped
vellum keys status          # shows progress and the key in use
```
