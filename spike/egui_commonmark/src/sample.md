# spike: egui_commonmark coverage check

Verify rendering of every element required by `docs/preview.md` 2.1 / 2.2.

## 1. CommonMark basics

This is a paragraph with *emphasis*, **strong**, `inline code`,
and a [hyperlink to the spec](https://spec.commonmark.org/0.31.2/).

> Block quote.
>
> Nested paragraph inside the quote.

Ordered list:

1. First
2. Second
   1. Nested
3. Third

Unordered list:

- alpha
- beta
- gamma

---

## 2. Code blocks (with `better_syntax_highlighting`)

```rust
fn main() {
    let x = 42;
    println!("hello {x}");
}
```

```python
def greet(name: str) -> str:
    return f"hello {name}"
```

```
plain text (no language)
```

## 3. GFM table

| Column A | Column B | Column C |
|----------|---------:|:--------:|
| left     |    right |  center  |
| **bold** | `code`   | _em_     |

## 4. Task list

- [x] done item
- [ ] pending item
- [x] another done item

## 5. Strikethrough

This text has ~~strikethrough~~ applied to it.

## 6. Autolink (bare URL)

Visit https://github.com/emilk/egui directly.

## 7. Footnote

Here is a sentence with a footnote.[^note]

[^note]: This is the footnote body.

## 8. Limited raw HTML

A line break here:<br>continues here.

## 9. Image (relative path)

![rust logo placeholder](./missing.png)
