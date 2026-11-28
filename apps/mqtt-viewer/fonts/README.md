# Fonts

| File | What it is | License |
|---|---|---|
| `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | [Inter](https://rsms.me/inter/) 4, Latin subset as published by Fontsource (`@fontsource/inter` 5.2.5). | SIL OFL 1.1, `Inter-LICENSE.txt` |
| `Phosphor.ttf` | [Phosphor Icons](https://phosphoricons.com) (regular weight), cut down to the icons in `src/ui/fonts.rs`. | MIT, `Phosphor-LICENSE.txt` |

egui's built-in fonts stay behind these as fallbacks for any character they lack.

## Adding an icon

Look up the icon's code point (the `codepoints.rs` of the `egui-phosphor` crate lists them), add a constant to
`src/ui/fonts.rs` and cut the font again from the full `Phosphor.ttf` of that crate, with
[fonttools](https://github.com/fonttools/fonttools) and every code point in `fonts.rs`:

```sh
pyftsubset Phosphor.ttf --no-hinting --output-file=apps/mqtt-viewer/fonts/Phosphor.ttf \
  --unicodes="$(grep -oE 'u\{[0-9A-F]+\}' apps/mqtt-viewer/src/ui/fonts.rs | sed -E 's/u\{(.*)\}/U+\1/' | paste -sd, -)"
```

A test fails while an icon in `fonts.rs` is missing from the font.
