{
  mkShell,
  rust-bin,
  mold,
  nodejs_24,
  chromium,
}:

mkShell {
  packages = [
    rust-bin.stable.latest.minimal
    mold
    nodejs_24
    chromium
  ];

  # Playwright ships an FHS-built Chromium that cannot load its shared
  # libraries on NixOS (no glib/gtk on the standard paths), so the browser
  # tests run against the Nix-provided Chromium instead.
  PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH = "${chromium}/bin/chromium";
}
