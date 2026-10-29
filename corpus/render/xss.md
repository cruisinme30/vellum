# Hostile input

Raw HTML is text: <script>alert(1)</script> and <img src=x onerror=alert(1)>.

[javascript link](javascript:alert(1)) and [data link](data:text/html,<b>x</b>) and [vbscript](VBScript:msgbox).

![data image](data:image/svg+xml,<svg onload=alert(1)>) ![js image](javascript:alert(1))

[[<b>wiki</b>]] and @"><script> and [zero width](java&#8203;script:alert(1)).

Text with a bidi override ‮reversed‬ and a control char .

## Heading {#"><script>alert(1)</script> .x"onmouseover=y}

::: callout {.vl-callout-crit onclick="alert(1)" style="color:red"}
Attributes do not leak.
:::

::: embed https://evil.example/frame
Not an allowed embed.
:::

::: embed javascript:alert(1)
:::

::: link javascript:alert(1)
Link card.
:::

```chart
title: <script>alert(1)</script>
labels: ["<img src=x onerror=alert(1)>", B]
values: [1, 2]
```

```mermaid
flowchart LR
  A["<img src=x onerror=alert(1)>"] --> B[click]
  click B "javascript:alert(1)"
```

```image
src: javascript:alert(1)
alt: bad
```

```math
\href{javascript:alert(1)}{x} + \unknowncommand
```

```table
"<script>",x
"=cmd|' /C calc'!A0",2
```

```html
<script>alert("code fences are text")</script>
```
