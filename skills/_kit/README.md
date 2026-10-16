# The page kit

Momputer's look for the skills' pages: one CSS file, one JS file, mom, and three fonts. `sync.py` copies all of it into each page, so a page stays one file that works offline. `disk-space-report` is the reference page.

## Use it in a page

Put the markers in the template, then run `python -I skills/_kit/sync.py`:

```html
<style id="kit-css">/*@kit-css*/ /*@end*/</style>
<!--@mom--><!--@end-->                      (optional: mom's face)
<script>
/*@kit-js*/ /*@end*/
// the page's own script, using Kit.*
</script>
```

Edit the kit here, never a page's copy. `sync.py --check` (run by `tests/run_all.py`) fails if a page holds an old copy.

## What's in it

| File | What |
| --- | --- |
| `kit.css` | Light and dark tokens, the data colours `--c1` to `--c8`, and parts: masthead with mom and her speech bubble, sticky section bar with search, cards, LEDs, swatches and tags, dot meter, bars, buttons, switches, tabs, rows, folds, tooltip, toast, bottom dock, reveal motion, print |
| `kit.js` | `Kit.el` / `Kit.svg` (nodes from `textContent` only), `bytes`, `count`, `pct`, `plural`, `store` (safe `localStorage`), `animate`, `tween`, `countUp`, `reveal`, `whenShown`, `smooth` (folds), `swap` (view transitions), `tip`, `toast`, `copy`, `flash`, `theme`, `key`, `spy`, `blink`, `start` |
| `mom.svg` | Mom's face, the same as the app's |
| `fonts/` | Geist, Geist Mono and Doto, SIL Open Font License 1.1 (`OFL.txt`) |

## Rules it keeps

- No network, no library, no `innerHTML`.
- Colour is for telling data apart, in fixed order, always next to a name. Grey for everything else; red and green only for state.
- Every animation plays once and ends. Reduced motion turns them off. Content below the first screen is never hidden waiting for one.
