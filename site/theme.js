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
  var system = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;

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
      // Private windows may refuse storage: the toggle still works for this page.
    }
  }

  function current() {
    return root.dataset.theme || stored() || (system && system.matches ? "dark" : "light");
  }

  // Before first paint: an explicit choice wins over the system setting.
  // mdBook reads its own key a moment later, so keep it in step; with no
  // choice, clear it so mdBook follows the system too.
  var choice = stored();
  if (choice) root.dataset.theme = choice;
  // The toggle is shown from first paint, so it never shifts the bar in
  // later; without JavaScript it stays hidden, since it could not work.
  root.setAttribute("data-theme-toggle", "");
  try {
    if (choice) localStorage.setItem("mdbook-theme", choice === "dark" ? "navy" : "light");
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
    var book = document.getElementById("mdbook-theme-" + (next === "dark" ? "navy" : "light"));
    if (book) book.click();
    render();
  }

  function wire() {
    var buttons = document.querySelectorAll(".theme-toggle");
    for (var i = 0; i < buttons.length; i++) buttons[i].addEventListener("click", toggle);
    render();
  }

  if (system && system.addEventListener) system.addEventListener("change", render);
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", wire);
  else wire();
})();
