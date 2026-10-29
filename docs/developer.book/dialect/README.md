---
id: 01M63CFE64ZRPDA1PGS1AMF6GK
title: Vellum Markdown grammar
vellum: 1
---

# Vellum Markdown grammar

Vellum Markdown, dialect version 1, is CommonMark 0.31 with the GitHub
Flavored Markdown extensions, plus five additions. Raw HTML is not part
of the dialect: HTML in a file is shown as text, never rendered.

| Addition | Syntax |
| --- | --- |
| Front matter | A YAML block between `---` lines at the start of the file |
| Containers | `:::` fences in the style of Pandoc's fenced divs |
| Special fences | Code fences whose language names a block, such as `chart` |
| Math | `$…$` inline and `$$` blocks, in LaTeX syntax |
| Inline extensions | Highlight, sub and superscript, wikilinks, mentions, dates, issue keys, block IDs |

The [user guide](../../user.book/writing/README.md) shows every block
rendered and as source.

## Grammar {#grammar}

The additions, in EBNF. Everything not defined here is CommonMark and
GFM.

```ebnf
document     = [ front-matter ] , { block } ;
front-matter = "---" , newline , yaml , "---" , newline ;

container    = fence-open , { block } , fence-close ;
fence-open   = colons , " " , name , [ " " , argument ] , [ " " , attributes ] , newline ;
fence-close  = colons , newline ;               (* same number of colons *)
colons       = ":::" , { ":" } ;                (* outer containers use more *)
name         = letter , { letter | digit | "-" } ;

special      = backticks , name , [ " " , words ] , [ " " , attributes ] , newline ,
               { line } , backticks , newline ;

attributes   = "{" , attribute , { " " , attribute } , "}" ;
attribute    = "#" , ident | "." , ident | key , "=" , value ;
value        = bare | '"' , { char - '"' } , '"' ;

block-id     = " ^blk-" , 8 * base32 ;         (* at the end of a block's last line *)
wikilink     = "[[" , title , [ "|" , label ] , "]]" ;
mention      = "@" , handle , [ "/" , handle ] ;
date         = yyyy , "-" , mm , "-" , dd | "@today" | "@" , relative-day ;
issue-key    = upper , { upper } , "-" , digit , { digit } ;
highlight    = "==" , text , "==" ;
subscript    = "~" , text , "~" ;
superscript  = "^" , text , "^" ;
```

## Known blocks {#blocks}

::: facts

- Containers: `info`, `warn`, `good`, `crit`, `tip`, `note`, `details`, `columns`, `tabs`, `toc`, `stats`, `facts`, `cards`, `steps`, `progress`, `link`, `embed`, `pages`
- Fences: `chart`, `timeline`, `roadmap`, `risk-matrix`, `board`, `table`, `architecture`, `mermaid`, `d2`, `plantuml`, `graphviz`, `excalidraw`, `drawio`, `calculator`, `image`, `collection`, `math`

:::

## Fallbacks {#fallbacks}

An unknown container renders as a plain box with its contents; an
unknown fence renders as a code block. A block that fails to parse
renders as its source with a diagnostic. No input makes a page fail to
render.

## Canonical form {#canonical}

The serializer writes one canonical form: `-` bullets, `*` emphasis,
`**` strong, fenced code with backticks, attributes in the order id,
classes, then keys. Parsing a canonical file and serializing it again
gives the same bytes; the golden corpus checks this for every block.

## Links {#links}

Wikilinks and links to other pages are stored as `vellum://doc/<id>`
when saved, so renames never break them. A link with the text `§` or no
text renders as the target's section number and title in a book.
