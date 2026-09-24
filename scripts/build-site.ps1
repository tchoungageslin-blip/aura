# build-site.ps1 - assemble the static Aura website into site/dist/
#
#   - copies hand-written pages + icon assets
#   - generates errors.html from docs/src/errors.md (single source)
#   - builds docs/src/ with mdBook into dist/docs/ when mdbook is on PATH
#     (otherwise dist ships a docs/ page pointing at `aura doc`)
#   - link-checks every local href/src in the produced files
#
# Usage:  scripts/build-site.ps1   ->  site/dist/
# NOTE: keep this file pure ASCII - PowerShell 5.1 misdecodes UTF-8
# scripts without a BOM.
param([string]$Out = "site/dist")

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$out = [System.IO.Path]::GetFullPath((Join-Path $root $Out))
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null

# --- 1. static pages + assets -------------------------------------------------
foreach ($f in 'index.html', 'download.html', 'learn.html', 'style.css', 'aura.js') {
    Copy-Item "$root/site/$f" "$out/$f"
}
foreach ($f in 'favicon.png', 'icon.png', 'social.png', 'aura.svg') {
    Copy-Item "$root/assets/icon/$f" "$out/$f"
}

# --- 2. errors.html generated from docs/src/errors.md -------------------------
$errorsMd = [IO.File]::ReadAllText("$root/docs/src/errors.md")
$rows = [System.Text.StringBuilder]::new()
foreach ($m in [regex]::Matches($errorsMd, '(?m)^### (E\d{4}) \u2014 (.+)$')) {
    $code = $m.Groups[1].Value; $title = $m.Groups[2].Value
    # first paragraph after the heading = short description
    $after = $errorsMd.Substring($m.Index + $m.Length)
    $body = ([regex]::Split($after, "`r?`n`r?`n"))[0] -replace '`([^`]+)`', '<code>$1</code>'
    $body = $body.Trim() -replace "`r?`n", ' '
    [void]$rows.AppendLine(
        "<tr id=`"$code`"><td><code>$code</code></td><td><b>$title</b><br><span class=`"dim`">$body</span></td></tr>")
}
$nav = @'
<nav>
  <a class="brand" href="index.html"><img src="icon.png" alt="A" width="28" height="28"> Aura</a>
  <div>
    <a href="learn.html">Learn</a>
    <a href="docs/">Docs</a>
    <a href="errors.html">Errors</a>
    <a href="download.html" class="cta">Download</a>
  </div>
</nav>
'@
$errorsHtml = @"
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Error Index - Aura</title>
<link rel="icon" type="image/png" href="favicon.png">
<link rel="stylesheet" href="style.css">
<style>.dim{color:var(--dim)} td code{white-space:nowrap}</style>
</head>
<body>
$nav
<article>
<h1>Error Index</h1>
<p>Every Aura diagnostic carries a stable code. Offline:
<code>aura doc E2101</code> prints the full entry; <code>aura doc errors</code>
prints the whole index. Detailed text + fixes:
<a href="docs/errors.html">docs/errors.html</a>.</p>
<table>
<tr><th>Code</th><th>Meaning</th></tr>
$rows
</table>
</article>
<footer><p>Generated from <code>docs/src/errors.md</code> - the same source the CLI embeds.</p></footer>
<script src="aura.js"></script>
</body>
</html>
"@
Set-Content "$out/errors.html" $errorsHtml -Encoding UTF8

# --- 3. docs/ via mdBook ------------------------------------------------------
$docsOut = Join-Path $out 'docs'
$mdbook = Get-Command mdbook -ErrorAction SilentlyContinue
if ($mdbook) {
    & mdbook build "$root/docs" --dest-dir $docsOut | Out-Null
    Write-Host "mdbook -> docs/"
} else {
    New-Item -ItemType Directory -Force $docsOut | Out-Null
    $fallback = @'
<!DOCTYPE html><html><head><meta charset="utf-8"><title>Aura Docs</title>
<link rel="stylesheet" href="../style.css"></head><body>
<nav>
  <a class="brand" href="../index.html"><img src="../icon.png" alt="A" width="28" height="28"> Aura</a>
  <div>
    <a href="../learn.html">Learn</a>
    <a href="../errors.html">Errors</a>
    <a href="../download.html" class="cta">Download</a>
  </div>
</nav>
<article><h1>Documentation</h1>
<p>This build was produced without mdBook. The same text ships inside the
compiler - run:</p>
<pre><code>aura doc           # list topics
aura doc stdlib    # standard library reference
aura doc E2101     # one error code</code></pre>
<p>Or read <a href="https://github.com/aura-lang/aura/tree/master/docs/src">docs/src on GitHub</a>.</p>
</article></body></html>
'@
    Set-Content "$docsOut/index.html" $fallback -Encoding UTF8
    Write-Warning "mdbook not found - docs/ contains a fallback page"
}

# --- 4. link check ------------------------------------------------------------
$bad = 0
Get-ChildItem $out -Recurse -Filter *.html | ForEach-Object {
    $html = Get-Content $_.FullName -Raw
    $dir = $_.DirectoryName
    foreach ($m in [regex]::Matches($html, '(?:href|src)="([^"#]+?)"')) {
        $href = $m.Groups[1].Value
        if ($href -match '^(https?:)?//' -or $href -match '^(mailto:|data:)') { continue }
        $target = Join-Path $dir ($href -replace '/', '\')
        if ($href.EndsWith('/')) { $target = Join-Path $target 'index.html' }
        elseif (-not [IO.Path]::GetExtension($target)) { $target = Join-Path $target 'index.html' }
        if (-not (Test-Path $target)) {
            # docs/*.html only exist when mdbook produced the book.
            if ($href -like 'docs/*' -and -not $mdbook) {
                Write-Host "  (needs mdbook) $($_.Name) -> $href"
            } else {
                Write-Warning "$($_.Name): broken link -> $href"; $bad++
            }
        }
    }
}
if ($bad -gt 0) { Write-Error "$bad broken link(s)" }
Write-Host "site built -> $out"
