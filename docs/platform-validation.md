# Platform validation

The CI matrix validates every push on Linux, macOS, and Windows. It builds,
lints, runs all tests, exercises the native PTY and terminal parser, and creates
the platform release archive. Linux additionally starts the complete GUI under
both X11 (Xvfb) and Wayland (headless Weston). The GUI smoke command exits only
after creating the window, renderer, IME context, and initial shell session.

Before publishing a release candidate, run the following interactive checks on
physical machines. Record the OS version, display scale, keyboard layout, and
result in the release issue.

## Linux Wayland and X11

- Start `toyoterm`, type ASCII and non-ASCII text, and run `printf '\e[31mred\e[0m\n'`.
- Compose one accented character and enter Japanese text through an IME.
- Copy and paste between toyoterm and a native desktop application.
- Set `config.window.opacity = 0.8`, reload, and confirm compositor transparency.
- Split a pane, resize the window, then close the application with shells alive.
- Install from the archive, launch it from the desktop menu and shell, install
  the same release again, then run the packaged uninstaller. Confirm no installed
  files remain and the user configuration is preserved.

Run the list once in a Wayland session and once with `WINIT_UNIX_BACKEND=x11` in
an X11 session. CI covers startup for both display protocols on every push.

## macOS

- Launch `toyoterm.app` and exercise the default shell PTY.
- Verify Command shortcuts, Option-modified input, dead keys, and Japanese IME.
- Copy and paste to TextEdit; check rendering at 1x and Retina scale.
- Repeat pane split, resize, reload, and shutdown checks.
- Open the DMG, drag the app to Applications, launch it, replace it with the same
  release once, and remove it. Confirm the `.tar.gz` contains the same version.

## Windows

- Start both PowerShell and `cmd.exe`; verify output, input, resize, and exit.
- Verify Ctrl shortcuts, AltGr, dead keys, and a Windows IME.
- Copy and paste to Notepad; check 100%, 150%, and 200% DPI.
- Repeat pane split, reload, and shutdown checks.
- Exercise portable zip startup, the default per-user installer, upgrade, and
  uninstaller. Confirm the user PATH entry and Start Menu shortcut are both
  added and removed.

Windows PTY code is confined to `crates/toyoterm-pty/src/windows.rs`; the Unix backend remains
behind `cfg(unix)` in `crates/toyoterm-pty/src/lib.rs`. The rest of the application uses `Pty`,
`PtySession`, `PtyCommand`, and `PtySize`, preventing ConPTY details from leaking
into mux, terminal, renderer, or scripting code.
