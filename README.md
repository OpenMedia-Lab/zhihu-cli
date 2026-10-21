# zhihu-cli

A lightweight, cross-platform Zhihu command-line client in Rust — built for agent pipelines (postdeck) but usable by humans too.

JSON in, JSON out. One binary, no runtime deps.

## Install

Grab a binary for macOS / Linux / Windows from [Releases](https://github.com/OpenMedia-Lab/zhihu-cli/releases), or build from source:

```bash
cargo build --release
```

## Auth

Three ways in, cookies are saved to `~/.config/zhihu-cli/cookies.json` (0600):

```bash
zhihu login                    # QR code — scan with the Zhihu app
zhihu login --browser          # extract zhihu.com cookies from an installed browser
zhihu login --cookie 'z_c0=…; _xsrf=…; d_c0=…'   # or "--cookie -" to read stdin

zhihu status                   # check the session
zhihu logout
```

## Commands

```bash
# read
zhihu whoami                          # current user profile
zhihu my-articles [--limit N]         # your own articles (pagination via --offset)
zhihu user-articles <url_token>       # anyone's articles
zhihu user-answers  <url_token>       # anyone's answers
zhihu user-pins     <url_token>       # anyone's pins (想法)
zhihu read <id|url>                   # article / answer / question / pin
zhihu search <kw> [--kind general|topic|people]
zhihu topics <kw>                     # resolve keyword → topic ids
zhihu hot [--limit N]                 # 热榜

# write
zhihu upload <image>                  # → zhimg URL (OSS flow under the hood)
zhihu upload --url <remote_url>       # rehost a remote image server-side
zhihu article "标题" --content-file body.html [-i img]… [--cover cover.png] [-t topic]…
                                      # saves a DRAFT — nothing goes public
zhihu article "标题" --content "<p>…" --publish
                                      # explicit --publish goes live
zhihu pin "标题" -c "正文" -i img…     # 想法 — publishes immediately, no draft mode
zhihu draft <id>                      # read a draft back (verify the preview)
zhihu delete-article <id>             # works on drafts too
zhihu delete-pin <id>
```

## Draft-first publishing

Zhihu has no "private note" — the preview surface is the **draft box**. `zhihu article` without `--publish` creates the draft, fills title/content/images, and returns an `edit_url` you can open in a browser to review and publish manually:

```json
{ "id": "2092…", "state": "draft", "edit_url": "https://zhuanlan.zhihu.com/p/…/edit" }
```

Pins (想法) are the exception — Zhihu offers no pin preview, so `zhihu pin` publishes immediately.

## Known limits

- Image processing can lag draft readback. Verify the saved content before publishing; a 1×1 test image was stripped while a 64×64 image survived publishing.

- Article **topics** are resolved through the editor's picker and bound to the draft with the native topics endpoint. `-t` / `--topic` accepts repeated names or numeric IDs. Cover images use `--cover` and persist as the article's title image.
- `--browser` extraction needs a desktop browser where zhihu.com is logged in (feature `browser-cookies`, on by default; `--no-default-features` for a slim build).
- QR login and `pin` are implemented but less battle-tested than the cookie/draft paths.

## License

MIT

## Editing articles

```bash
zhihu edit-article <id> --title "Revised title"
zhihu edit-article <id> --content-file body.html -i illustration.png
zhihu edit-article <id> --cover cover.png -t "Rust（编程语言）"
zhihu edit-article <id> --publish
```

Edits target the same article's draft. Omitted title/content/cover/topics are preserved.
Provided topics replace the existing topic list. Title/content fields are left out
of the PATCH, preserving their existing values. Images append to the current
body when no replacement body is supplied. Published articles can be edited via
their draft and published again with `--publish`. Explicit empty content clears
the body. Edit results report the server article state and `publish_requested`;
a published article can retain that state while its draft is being edited.
Runtime failures emit JSON to stderr and exit with status 1; argument
usage errors follow clap's standard status 2.

Development checks: `cargo fmt --check`, `cargo test --no-default-features`,
`cargo check`. The cookie store must never be checked into version control.
