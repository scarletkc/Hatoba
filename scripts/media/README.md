# Update the screenshots, promo film, and social preview

The scripts in this folder capture the screenshots and the promo film's product shots from the browser frontend's mock backend. The website's English pages use the screenshots in `docs/media`, and its Chinese and Japanese pages the ones in `apps/site/src/assets/screenshots`. The promo film is in `docs/media` in English and Chinese, and the README opens with the English one. `?ai=showcase` in `apps/desktop/src/ipc/mock/ai.ts` scripts the AI conversation they show. They need Python 3 and ffmpeg with libx264. Start `pnpm dev`, then from the repository root:

```sh
pip install -r scripts/media/requirements.txt
playwright install chromium
python scripts/media/screenshots.py                  # ai-assistant, ai-approval, dashboard, and sync-deploy PNGs
python scripts/media/screenshots.py --locale zh-CN   # the same in Chinese, and with --locale zh-TW or ja in Traditional Chinese or Japanese
python scripts/media/promo/capture.py                # the app's states that the film shows, in English and Chinese
python scripts/media/promo/render.py --lang en --music track.mp3 --music-offset 0.37   # hatoba-promo-en.mp4, and --lang zh
```

`--out DIR` writes somewhere else for a look before replacing the files. GitHub doesn't play videos from the repository, so the README plays the English film from an attachment: `render.py --small`, with the same music options, writes a copy under the 10 MB that GitHub takes, and dragging it into a comment box on GitHub gives its link. The film is an animation in `scripts/media/promo/index.html`, which plays in a browser when the repository root is served (`python -m http.server`, then `/scripts/media/promo/index.html`), and `scripts/media/promo/sfx.py` synthesizes its sound effects. The music isn't in the repository: the films use gritz16's "Meet Me at the Lobby" (120 BPM), and `--music-offset 0.37` puts its downbeats on the film's beats.

The repository's social preview, the image that links to the repository show, is `docs/media/social-preview.png`: the website's wordmark, tagline, and sea beside the `ai-assistant` screenshot, laid out in `scripts/media/social-preview/index.html`. After either changes, `python scripts/media/social-preview/render.py` writes it again without `pnpm dev`, and it goes to GitHub by hand, under **Social preview** in the repository's general settings.
