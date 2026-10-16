# The page kit

The app's look for the skills' pages: one CSS file, one JS file, mom, and three fonts. `sync.py` copies all of it into each page, so a page stays one file that works offline. `disk-space-report` is the reference page.

## Use it in a page

Put the markers in the template, then run `python -I skills/_kit/sync.py`:

```html
<style id="kit-css">/*@kit-css*/ /*@end*/</style>
<div class="screen" id="mom"><!--@mom--><!--@end--></div>   (mom's face)
<script>
/*@kit-js*/ /*@end*/
// the page's own script, using Kit.*
</script>
```

Edit the kit here, never a page's copy. `sync.py --check` (run by `tests/run_all.py`) fails if a page holds an old copy.

## What's in it

| File | What |
| --- | --- |
| `kit.css` | The app's light and dark tokens, the lights `--c1` to `--c6`, and its parts: title bar, mom's card (`.note`, `.screen`, `.big`, `.line`), section heads, cards (`.group`) of rows with dividers, lights (`.led`, `.led.c`, `.who`), dot meters, buttons (`.primary`, `.secondary`, `.link-btn`), ticks, sub tabs, the section bar with search, tooltip, toast, bottom dock, print |
| `kit.js` | `Kit.el` / `Kit.svg` (nodes from `textContent` only), `bytes`, `count`, `pct`, `plural`, `store` (safe `localStorage`), `meter` and `stack` (dot meters), `animate`, `tween`, `countUp`, `reveal` (tells you when something comes into view; hides nothing), `whenShown`, `smooth` (folds), `swap` (view transitions), `tip`, `toast`, `copy`, `flash`, `theme`, `key`, `spy`, `blink`, `start` |
| charts in `kit.js` | `Kit.columns(labels, series, opts)` dot columns over time, stacked by series; `Kit.heat(rowLabels, colLabels, values, opts)` dots sized by value (weekday by hour); `Kit.waffle(parts, opts)` 100 dots split into shares, with its key; `Kit.bars(rows, opts)` ranked rows with a meter; `Kit.table(cols, rows, opts)` sortable, with "Show more"; `Kit.facts(items)` a few dot-matrix figures; `Kit.insights(list)` Mom's take; `Kit.led(color)`, `Kit.legend(items)`, `Kit.share(values, n)`. Every mark has a tooltip; text is ink, colour is only in the dots |
| `mom.svg` | Mom's face, the same as the app's |
| `fonts/` | Geist, Geist Mono and Doto, SIL Open Font License 1.1 (`OFL.txt`) |

## Rules it keeps

- No network, no library, no `innerHTML`.
- Colour is a light: one per thing, in fixed order, next to its name, never red or green. Red and green lights are for state. No coloured borders, stripes, glows or gradients. The faint dot paper behind the page is the website's own.
- Content shows at once. Motion answers what someone did, eases out without bouncing, and never loops. Reduced motion turns it off.
- Secondary text passes 4.5:1 on its background.
