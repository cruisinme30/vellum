// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only
//
// The light and dark toggle on every page of the website (ADR 0027, D308).
// The site follows the system setting until the reader presses the toggle;
// the choice is then kept in this browser for the whole site (landing page,
// docs home and every help book) and nothing is sent anywhere.
//
// Loaded in <head> without defer, so the theme is set before first paint.
// In a help book it also drives mdBook's own theme switcher: "light" is
// mdBook's light theme and "dark" its navy theme, both recoloured by book.css.
(function () {
  "use strict";
  var KEY = "vellum-site-theme";
  var root = document.documentElement;
  var system = window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

  function stored() {
    try {
      var value = localStorage.getItem(KEY);
      return value === "light" || value === "dark" ? value : null;
    } catch (e) {
      return null;
    }
  }

  function store(value) {
    try {
      localStorage.setItem(KEY, value);
      localStorage.setItem("mdbook-theme", value === "dark" ? "navy" : "light");
    } catch (e) {
      // Private windows may refuse storage: the toggle still works for this
      // page.
    }
  }

  function current() {
    return (
      root.dataset.theme ||
      stored() ||
      (system && system.matches ? "dark" : "light")
    );
  }

  // With no choice made, follow the system when it changes.
  function follow() {
    if (stored()) return;
    root.dataset.theme = system.matches ? "dark" : "light";
    render();
  }

  // Before first paint: an explicit choice wins over the system setting.
  // mdBook reads its own key a moment later, so keep it in step; with no
  // choice, clear it so mdBook follows the system too.
  // data-theme is always set, to the choice or else the system setting, so
  // stylesheets that only follow the attribute (the rendered diagrams and
  // callouts in the help books, ADR 0071) match the rest of the page.
  var choice = stored();
  root.dataset.theme = choice || (system && system.matches ? "dark" : "light");
  // The toggle is shown from first paint, so it never shifts the bar in
  // later; without JavaScript it stays hidden, since it could not work.
  root.setAttribute("data-theme-toggle", "");
  try {
    if (choice)
      localStorage.setItem(
        "mdbook-theme",
        choice === "dark" ? "navy" : "light",
      );
    else localStorage.removeItem("mdbook-theme");
  } catch (e) {
    // No storage: mdBook falls back to the system setting on its own.
  }

  function render() {
    var dark = current() === "dark";
    var buttons = document.querySelectorAll(".theme-toggle");
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].setAttribute("aria-pressed", dark ? "true" : "false");
    }
  }

  function toggle() {
    var next = current() === "dark" ? "light" : "dark";
    root.dataset.theme = next;
    store(next);
    // In a help book, let mdBook swap its classes and code highlighting.
    var book = document.getElementById(
      "mdbook-theme-" + (next === "dark" ? "navy" : "light"),
    );
    if (book) book.click();
    render();
  }

  // The site bar wraps when its text is long or the screen narrow; keep
  // --vb-height at its real height, so a help book starts its sidebar and
  // page below it (ADR 0071, D310). Set through the CSSOM, which the
  // site's Content Security Policy allows.
  // A help book also calls this straight after the bar is parsed, before
  // first paint, so a bar taller than usual never shifts the page later.
  var measured = false;
  function measureBar() {
    var bar = document.querySelector("header.vellum-site");
    if (!bar) return;
    root.style.setProperty("--vb-height", bar.offsetHeight + "px");
    if (measured || !window.ResizeObserver) return;
    measured = true;
    new ResizeObserver(function () {
      root.style.setProperty("--vb-height", bar.offsetHeight + "px");
    }).observe(bar);
  }
  window.vellumMeasureBar = measureBar;

  function wire() {
    var buttons = document.querySelectorAll(".theme-toggle");
    for (var i = 0; i < buttons.length; i++)
      buttons[i].addEventListener("click", toggle);
    measureBar();
    render();
  }

  if (system && system.addEventListener)
    system.addEventListener("change", follow);
  if (document.readyState === "loading")
    document.addEventListener("DOMContentLoaded", wire);
  else wire();
})();
