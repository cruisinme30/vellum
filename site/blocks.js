// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only
//
// Tabs for the rendered ::: tabs blocks in the help books (ADR 0071, D311).
// The renderer writes every tab as a titled section, one after another,
// which is also how the page reads and prints without this script. Here
// each block gets a row of tab buttons that shows one section at a time,
// following the WAI-ARIA tabs pattern: arrow keys, Home and End move
// between tabs. Nothing is stored or sent.
(function () {
  "use strict";

  function setUp(block, n) {
    var sections = [];
    for (var i = 0; i < block.children.length; i++) {
      if (block.children[i].classList.contains("vl-tab")) sections.push(block.children[i]);
    }
    if (sections.length < 2) return;
    var list = document.createElement("div");
    list.className = "vl-site-tablist";
    list.setAttribute("role", "tablist");
    var tabs = sections.map(function (section, i) {
      var title = section.querySelector(".vl-tab-title");
      var tab = document.createElement("button");
      tab.type = "button";
      tab.className = "vl-site-tab";
      tab.id = "vl-site-tab-" + n + "-" + i;
      tab.setAttribute("role", "tab");
      tab.textContent = title ? title.textContent : "Tab " + (i + 1);
      if (!section.id) section.id = "vl-site-panel-" + n + "-" + i;
      tab.setAttribute("aria-controls", section.id);
      section.setAttribute("role", "tabpanel");
      section.setAttribute("aria-labelledby", tab.id);
      if (title) title.classList.add("vl-site-tab-heading");
      list.appendChild(tab);
      return tab;
    });

    function select(index, focus) {
      tabs.forEach(function (tab, i) {
        var on = i === index;
        tab.setAttribute("aria-selected", on ? "true" : "false");
        tab.tabIndex = on ? 0 : -1;
        sections[i].hidden = !on;
      });
      if (focus) tabs[index].focus();
    }

    tabs.forEach(function (tab, i) {
      tab.addEventListener("click", function () {
        select(i, false);
      });
      tab.addEventListener("keydown", function (event) {
        var next = null;
        if (event.key === "ArrowRight") next = (i + 1) % tabs.length;
        else if (event.key === "ArrowLeft") next = (i - 1 + tabs.length) % tabs.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = tabs.length - 1;
        if (next === null) return;
        event.preventDefault();
        // mdBook turns the page on the arrow keys; inside a tab row they
        // move between tabs instead.
        event.stopPropagation();
        select(next, true);
      });
    });
    block.insertBefore(list, block.firstChild);
    block.classList.add("vl-site-tabs");
    select(0, false);
  }

  function wire() {
    var blocks = document.querySelectorAll(".vl-render .vl-tabs");
    for (var i = 0; i < blocks.length; i++) setUp(blocks[i], i);
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", wire);
  else wire();
})();
