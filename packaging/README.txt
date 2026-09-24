=====================================================================
  AURA — a compiled systems language            v0.1.0-alpha
=====================================================================

CONTENTS OF THIS PACKAGE
------------------------
  aura.exe           the compiler / toolchain CLI
  aura_runtime.lib   runtime library (needed to link executables)
  lld-link.exe       bundled linker — NO Rust or Visual Studio needed
  aura-lang-*.vsix   VS Code extension (optional)
  bench/             example programs
  docs/              the full documentation (markdown)
  licenses/          third-party license notices

REQUIREMENTS
------------
  Windows 10/11 64-bit. Nothing else — no Rust, no Visual Studio,
  no admin rights needed.

INSTALL
-------
  Easiest: run AuraSetup-*.exe instead (it does all of this for you).

  Manual:
    1. Copy this folder anywhere, e.g.  C:\tools\aura
    2. Add it to your PATH:
         PowerShell (one time, persists):
           [Environment]::SetEnvironmentVariable(
             "Path", "$env:Path;C:\tools\aura", "User")
    3. Open a NEW terminal and check:   aura --version

  Windows may show a SmartScreen warning on first run — the binaries
  are unsigned (alpha). Click "More info" -> "Run anyway".

FIRST STEPS
-----------
    aura new hello          # scaffold a project
    cd hello
    aura run                # interpret it
    aura build              # produce a native .exe in build/
    aura doc                # list doc topics
    aura doc vec_push       # builtin docs, offline
    aura test               # run the validation suite (if shipped)

VS CODE EXTENSION (optional)
----------------------------
    code --install-extension aura-lang-0.1.0.vsix
  gives you syntax highlighting, diagnostics, hover and formatting.

DOCUMENTATION
-------------
  docs/ in this package, or https://<repo>.github.io/aura online.

LICENSE
-------
  Aura is dual-licensed MIT OR Apache-2.0. lld-link is part of the
  LLVM project under Apache-2.0 with LLVM Exceptions (see licenses/).
