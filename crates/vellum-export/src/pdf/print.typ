// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only
//
// The print profile. It reads the export model from /flow.json and the
// theme from /theme.json, and never evaluates document text as markup:
// every string from the document is shown as text. Only the converted
// maths (built by vellum-export from a fixed symbol table) is evaluated.

#let flow = json("/flow.json")
#let theme = json("/theme.json")
#let c(name) = rgb(theme.colors.at(name))

#set document(title: flow.title, date: none)
#set page(
  paper: theme.paper,
  margin: (x: 2.2cm, top: 2.4cm, bottom: 2.4cm),
  header: context {
    if counter(page).get().first() > 1 {
      set text(size: 8pt, fill: c("muted"))
      flow.title
      h(1fr)
      [As of #flow.as_of]
    }
  },
  footer: context {
    set text(size: 8pt, fill: c("muted"))
    h(1fr)
    [#counter(page).display() / #counter(page).final().first()]
    h(1fr)
  },
)
#set text(font: theme.fonts.body, size: theme.size * 1pt, fill: c("text"), lang: theme.lang)
#set par(justify: false, leading: 0.62em, spacing: 1em)
#show raw: set text(font: theme.fonts.mono, size: 0.86em)
#show heading: set text(fill: c("heading"), weight: "bold")
#show heading.where(level: 1): set text(size: 1.6em)
#show heading.where(level: 2): set text(size: 1.32em)
#show heading.where(level: 3): set text(size: 1.15em)
#show heading: it => block(above: 1.4em, below: 0.7em, sticky: true, it)
#show link: set text(fill: c("primary"))
#set table(stroke: 0.5pt + c("outline"), inset: 6pt)

#let tones = theme.tones
#let tone-fg(t) = rgb(tones.at(t).fg)
#let tone-bg(t) = rgb(tones.at(t).bg)

#let span(s) = {
  if s.t == "text" {
    let body = if s.at("code", default: false) {
      box(fill: c("surface"), inset: (x: 2pt), outset: (y: 2pt), radius: 2pt, raw(s.text))
    } else { [#s.text] }
    if s.at("bold", default: false) { body = strong(body) }
    if s.at("italic", default: false) { body = emph(body) }
    if s.at("strike", default: false) { body = strike(body) }
    if s.at("highlight", default: false) { body = highlight(fill: rgb("#FFF3A3"), body) }
    if s.at("sub", default: false) { body = sub(body) }
    if s.at("sup", default: false) { body = super(body) }
    body
  } else if s.t == "link" {
    link(s.url, s.spans.map(span).join())
  } else if s.t == "xref" {
    let target = label(s.anchor)
    link(target, [#s.text])
    context [, p.~#counter(page).at(target).first()]
  } else if s.t == "image" {
    box(height: 1em, image("/" + s.asset, alt: s.alt))
  } else if s.t == "break" {
    linebreak()
  }
}
#let spans(xs) = xs.map(span).join()

#let check(done) = if done { sym.ballot.check } else { sym.ballot }

#let align-of(a) = if a == "right" { right } else if a == "center" { center } else { left }

#let table-node(b) = {
  let n = b.align.len()
  let cell(x) = {
    let shade = x.at("shade", default: none)
    if shade == none { table.cell(spans(x.spans)) } else {
      table.cell(fill: tone-bg(shade), spans(x.spans))
    }
  }
  if b.style == "facts" {
    table(
      columns: (auto, 1fr),
      stroke: (x, y) => (bottom: 0.5pt + c("outline")),
      align: (x, y) => align-of(b.align.at(x, default: "left")),
      ..b.rows.flatten().map(cell),
    )
  } else {
    table(
      columns: n,
      align: (x, y) => align-of(b.align.at(x, default: "left")),
      fill: (x, y) => if y == 0 and b.header.len() > 0 { c("surface") },
      ..if b.header.len() > 0 {
        (table.header(..b.header.map(h => table.cell(strong(spans(h.spans))))),)
      },
      ..b.rows.flatten().map(cell),
    )
  }
}

#let block-node(b) = {
  let blocks(bs) = bs.map(block-node).join()
  if b.t == "heading" {
    let number = b.at("number", default: none)
    let body = if number == none { spans(b.spans) } else { [#number#h(0.6em)#spans(b.spans)] }
    let anchor = b.at("anchor", default: none)
    let h = heading(level: b.level, outlined: b.outlined, bookmarked: b.outlined, body)
    if anchor == none { h } else { [#h#label(anchor)] }
  } else if b.t == "paragraph" {
    let anchor = b.at("anchor", default: none)
    let p = par(spans(b.spans))
    if anchor == none { p } else { [#p#label(anchor)] }
  } else if b.t == "quote" {
    block(
      inset: (left: 12pt, y: 4pt),
      stroke: (left: 2pt + c("outline")),
      text(fill: c("muted"), style: "italic", blocks(b.blocks)),
    )
  } else if b.t == "list" {
    let start = b.at("start", default: none)
    let tasks = b.items.any(i => i.at("check", default: none) != none)
    if tasks {
      stack(spacing: 0.6em, ..b.items.map(i => grid(
        columns: (1.4em, 1fr),
        check(i.at("check", default: false)),
        blocks(i.blocks),
      )))
    } else if start == none {
      list(..b.items.map(i => blocks(i.blocks)))
    } else {
      enum(start: start, ..b.items.map(i => blocks(i.blocks)))
    }
  } else if b.t == "code" {
    block(
      width: 100%,
      fill: c("surface"),
      inset: 8pt,
      radius: 4pt,
      raw(b.text, lang: if b.lang == "" { none } else { b.lang }, block: true),
    )
  } else if b.t == "math" {
    let source = b.at("typst", default: none)
    if source == none {
      block(fill: c("surface"), inset: 8pt, radius: 4pt, raw(b.tex, block: true))
    } else {
      math.equation(block: true, eval(source, mode: "math"))
    }
  } else if b.t == "table" {
    table-node(b)
  } else if b.t == "rule" {
    line(length: 100%, stroke: 0.5pt + c("outline"))
  } else if b.t == "callout" {
    block(
      width: 100%,
      fill: tone-bg(b.tone),
      stroke: (left: 3pt + tone-fg(b.tone)),
      inset: 10pt,
      radius: (right: 4pt),
      {
        box(circle(radius: 5pt, fill: tone-fg(b.tone)))
        h(0.5em)
        text(weight: "bold", fill: tone-fg(b.tone), tones.at(b.tone).label)
        v(0.3em)
        blocks(b.blocks)
      },
    )
  } else if b.t == "titled" {
    block(sticky: true, below: 0.5em, text(weight: "bold", size: 1.05em, spans(b.title)))
    blocks(b.blocks)
  } else if b.t == "columns" {
    grid(columns: (1fr,) * b.columns.len(), gutter: 14pt, ..b.columns.map(blocks))
  } else if b.t == "stats" {
    grid(
      columns: (1fr,) * calc.max(1, calc.min(4, b.tiles.len())),
      gutter: 8pt,
      ..b.tiles.map(t => block(
        width: 100%,
        fill: c("surface"),
        inset: 10pt,
        radius: 6pt,
        {
          text(size: 1.6em, weight: "bold", t.value)
          linebreak()
          text(fill: c("muted"), t.label)
          if t.delta != "" {
            linebreak()
            let trend = t.at("trend", default: none)
            text(size: 0.9em, fill: if trend == none { c("muted") } else { tone-fg(trend) }, t.delta)
          }
        },
      )),
    )
  } else if b.t == "cards" {
    grid(
      columns: (1fr,) * calc.max(1, calc.min(3, b.cards.len())),
      gutter: 8pt,
      ..b.cards.map(k => block(
        width: 100%,
        stroke: 0.5pt + c("outline"),
        inset: 10pt,
        radius: 6pt,
        {
          if k.icon != "" { [#k.icon ] }
          strong(k.title)
          linebreak()
          spans(k.spans)
        },
      )),
    )
  } else if b.t == "progress" {
    stack(spacing: 0.7em, ..b.bars.map(bar => grid(
      columns: (35%, 1fr, 3em),
      gutter: 8pt,
      align: horizon,
      bar.label,
      box(width: 100%, height: 6pt, radius: 3pt, fill: c("surface"),
        box(width: bar.percent * 1%, height: 6pt, radius: 3pt, fill: c("primary"))),
      align(right)[#calc.round(bar.percent) %],
    )))
  } else if b.t == "link_card" {
    block(
      width: 100%,
      stroke: 0.5pt + c("outline"),
      inset: 10pt,
      radius: 6pt,
      {
        link(b.url, strong(b.title))
        if b.text != "" { linebreak(); b.text }
        linebreak()
        text(size: 0.85em, fill: c("muted"), b.url)
      },
    )
  } else if b.t == "figure" {
    let w = calc.min(b.width, 16cm / 1pt) * 1pt
    figure(
      image("/" + b.asset, width: w, alt: b.alt),
      caption: if b.caption == "" { none } else { [#b.caption] },
      supplement: none,
      numbering: none,
    )
  } else if b.t == "image" {
    figure(
      image("/" + b.asset, width: 100%, fit: "contain", alt: b.alt),
      caption: if b.caption == "" { none } else { [#b.caption] },
      supplement: none,
      numbering: none,
    )
  } else if b.t == "toc" {
    outline(title: [Contents], depth: b.depth, indent: auto)
  } else if b.t == "page_break" {
    pagebreak(weak: true)
  }
}

#if flow.layout == "book" {
  page(header: none, footer: none, {
    v(30%)
    text(size: 2.4em, weight: "bold", fill: c("heading"), flow.title)
    v(1em)
    text(fill: c("muted"), [As of #flow.as_of])
  })
} else if flow.show_title and flow.title != "" {
  block(below: 1.2em, text(size: 2em, weight: "bold", fill: c("heading"), flow.title))
}

#flow.blocks.map(block-node).join()
