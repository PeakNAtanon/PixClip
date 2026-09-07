# Repository Guidelines

## Project Structure & Module Organization

PixClip is a Python desktop media toolkit for Windows and Linux. Application code uses the standard library and Tkinter; media operations require external yt-dlp, FFmpeg, ffprobe, and Streamlink executables.

- `media_toolkit.py`: GUI/CLI entry point, media commands, tool installation, and process control.
- `pixclip_jobs.py`: persistent queue, scheduling, isolated download workers, and disk checks.
- `pixclip_ui.py`: queue/history controls, mascot states, and clip previews.
- `test_pixclip_jobs.py` and `test_pixclip_integration.py`: root-level test suites.
- `assets/`: mascot PNGs and Windows icon; `README-PixClip.md`: Thai user guide.
- `PixClip.bat`, `PixClip`, `Install-PixClip.bat`, `install.sh`, and associated scripts: Windows/Linux launchers and setup helpers.

## Build, Test, and Development Commands

Run commands from the project root; use `python3` where appropriate on Linux.

- `python media_toolkit.py`: launch the GUI; requires Tkinter and a desktop session.
- `python media_toolkit.py --cli`: open the terminal menu.
- `python -m py_compile media_toolkit.py pixclip_jobs.py pixclip_ui.py`: check syntax.
- `python -m unittest -v test_pixclip_jobs`: run offline regression tests.
- `python -c "import os,unittest; os.environ['PIXCLIP_INTEGRATION']='1'; unittest.main(module='test_pixclip_integration',verbosity=2)"`: run opt-in integration tests with media tools and a GUI display.

There is no compilation build step or configured formatter/linter. ZIP distributions must include all three application modules, launchers, assets, and documentation.

## Coding Style & Naming Conventions

Use four-space indentation, `snake_case` functions/variables, `PascalCase` classes, and `UPPER_SNAKE_CASE` constants. Follow nearby typing and docstring conventions. Prefer `pathlib.Path` and subprocess argument lists. Keep Tk updates on the GUI thread; use workers and queues for blocking work. Preserve the established mint/amber pixel theme.

## Testing Guidelines

Use `unittest`, `test_*.py` filenames, and `test_*` methods. Add behavioral regression tests for queue transitions, cancellation, scheduling, and file preservation. Integration tests use synthetic media and localhost; they do not prove public-site compatibility. No numeric coverage threshold is configured. Report platform-specific skips and distinguish Windows GUI checks from Linux CLI checks.

## Commit & Pull Request Guidelines

Use concise imperative commit subjects, matching the existing history (for example, `Fix scheduled recording cancellation` or `Add queue progress reporting`). PRs should explain behavior changes, reference relevant issues, list validation and platform limitations, and include screenshots for GUI changes. Exclude downloaded media, private history/logs, `__pycache__/`, `.venv/`, and machine-specific `.lnk` files from source changes.
