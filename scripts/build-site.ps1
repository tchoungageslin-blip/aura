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
foreach ($f in 'index.html', 'download.html', 'learn.html', '404.html', 'sitemap.xml', 'style.css', 'aura.js', 'i18n.js') {
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
    <a href="learn.html"><span class="len">Learn</span><span class="lfr">Apprendre</span></a>
    <a href="docs/">Docs</a>
    <a href="examples.html"><span class="len">Examples</span><span class="lfr">Exemples</span></a>
    <a href="errors.html"><span class="len">Errors</span><span class="lfr">Erreurs</span></a>
    <a href="download.html" class="cta"><span class="len">Download</span><span class="lfr">T&eacute;l&eacute;charger</span></a>
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
<meta name="description" content="Every Aura diagnostic code - what it means and how to fix it.">
<meta property="og:title" content="Aura Error Index">
<meta property="og:description" content="Every Aura diagnostic code, explained.">
<meta property="og:type" content="website">
<meta property="og:url" content="https://tchoungageslin-blip.github.io/aura/errors.html">
<meta property="og:image" content="https://tchoungageslin-blip.github.io/aura/social.png">
<meta name="twitter:card" content="summary">
<link rel="icon" type="image/png" href="favicon.png">
<link rel="stylesheet" href="style.css">
<style>.dim{color:var(--dim)} td code{white-space:nowrap}</style>
</head>
<body>
$nav
<article>
<h1><span class="len">Error Index</span><span class="lfr">Index des erreurs</span></h1>
<p><span class="len">Every Aura diagnostic carries a stable code. Offline:
<code>aura doc E2101</code> prints the full entry; <code>aura doc errors</code>
prints the whole index. Detailed text + fixes:
<a href="docs/errors.html">docs/errors.html</a>. Entries below mirror the
CLI output, so they stay in English.</span><span class="lfr">Chaque diagnostic Aura porte un code stable. Hors ligne :
<code>aura doc E2101</code> affiche l'entr&eacute;e compl&egrave;te ;
<code>aura doc errors</code> affiche tout l'index. Texte d&eacute;taill&eacute; + correctifs :
<a href="docs/errors.html">docs/errors.html</a>. Les entr&eacute;es ci-dessous
refl&egrave;tent la sortie du CLI, elles restent en anglais.</span></p>
<table>
<tr><th>Code</th><th><span class="len">Meaning</span><span class="lfr">Signification</span></th></tr>
$rows
</table>
</article>
<footer><p><span class="len">Generated from <code>docs/src/errors.md</code> - the same source the CLI embeds.</span><span class="lfr">G&eacute;n&eacute;r&eacute; depuis <code>docs/src/errors.md</code> - la m&ecirc;me source que le CLI embarque.</span></p></footer>
<script src="i18n.js"></script>
<script src="aura.js"></script>
</body>
</html>
"@
Set-Content "$out/errors.html" $errorsHtml -Encoding UTF8

# --- 2b. examples.html generated from testsuite/ -------------------------------
# Each scenario dir provides: a leading //-comment description (source of
# truth), .aura sources, and input files (stdin.txt, args.txt, ...).
# FR descriptions + category names live in site/examples-fr.json (UTF-8).
$fr = [IO.File]::ReadAllText("$root/site/examples-fr.json") | ConvertFrom-Json
$gh = 'https://github.com/tchoungageslin-blip/aura/tree/master/testsuite'
$categories = @(
    @{ lo = 1;  hi = 10; id = 'basics';   en = 'Language basics' },
    @{ lo = 11; hi = 20; id = 'algo';     en = 'Algorithms and data structures' },
    @{ lo = 21; hi = 28; id = 'io';       en = 'I/O, files and system' },
    @{ lo = 29; hi = 35; id = 'perf';     en = 'Performance' },
    @{ lo = 36; hi = 40; id = 'sci';      en = 'Scientific computing' },
    @{ lo = 41; hi = 45; id = 'apps';     en = 'Mini-applications' },
    @{ lo = 46; hi = 48; id = 'projects'; en = 'Projects' },
    @{ lo = 49; hi = 50; id = 'meta';     en = 'Meta' }
)
$catFr = @{}
foreach ($p in $fr.cat.PSObject.Properties) { $catFr[$p.Name] = $p.Value }
$descFr = @{}
foreach ($p in $fr.desc.PSObject.Properties) { $descFr[$p.Name] = $p.Value }

# Coverage gate: every testsuite scenario needs a FR description and every
# category a FR name - the build fails rather than silently emitting gaps.
$missingFr = @()
foreach ($dir in Get-ChildItem "$root/testsuite" -Directory | Sort-Object Name) {
    if ($dir.Name -notmatch '^\d+-') { continue }
    if (-not (Get-ChildItem $dir.FullName -Recurse -Filter *.aura)) { continue }
    if (-not $descFr.ContainsKey($dir.Name)) { $missingFr += $dir.Name }
}
foreach ($c in $categories) {
    if (-not $catFr.ContainsKey($c.id)) { $missingFr += "cat:$($c.id)" }
}
$orphans = @($descFr.Keys | Where-Object { -not (Test-Path "$root/testsuite/$_") })
if ($missingFr -or $orphans) {
    Write-Error ("examples-fr.json out of sync - missing: [{0}] orphans: [{1}]" -f
        ($missingFr -join ', '), ($orphans -join ', '))
}

function Html([string]$s) { [System.Net.WebUtility]::HtmlEncode($s) }

# Leading //-comment block of a .aura file = description text.
function Get-Desc([string]$file, [string]$fallback) {
    $desc = ''
    foreach ($line in [IO.File]::ReadLines($file)) {
        $t = $line.TrimEnd()
        if ($t.StartsWith('//')) {
            $desc += ' ' + $t.TrimStart('/').Trim()
        } elseif ($t -eq '') {
            if ($desc) { break }
        } else { break }
    }
    # strip the "NN-name - " prefix (em-dash, \u2014 in .NET regex)
    $desc = [regex]::Replace($desc.Trim(), '^[0-9]+-[a-z0-9-]+\s*\u2014?\s*', '')
    if (-not $desc) { $fallback }
    else { $desc }
}

$exRows = [System.Text.StringBuilder]::new()
foreach ($dir in Get-ChildItem "$root/testsuite" -Directory | Sort-Object Name) {
    $name = $dir.Name
    $n = [int]($name -split '-')[0]
    $auraFiles = Get-ChildItem $dir.FullName -Recurse -Filter *.aura | Sort-Object FullName
    if (-not $auraFiles) { continue }
    $descEn = Get-Desc $auraFiles[0].FullName "scenario $name"
    $descFrt = if ($descFr.ContainsKey($name)) { $descFr[$name] } else { $descEn }

    # inputs: everything that is not .aura / expected.* / aura.toml
    $inputs = Get-ChildItem $dir.FullName -Recurse -File |
        Where-Object { $_.Extension -ne '.aura' -and $_.Name -notlike 'expected*' -and $_.Name -ne 'aura.toml' } |
        Sort-Object FullName

    $srcLinks = foreach ($f in $auraFiles) {
        $rel = $f.FullName.Substring($dir.FullName.Length + 1) -replace '\\', '/'
        "<code>$rel</code>"
    }
    $ioNames = foreach ($f in $inputs) {
        "<code>" + ($f.FullName.Substring($dir.FullName.Length + 1) -replace '\\', '/') + "</code>"
    }

    # new category heading?
    foreach ($c in $categories) {
        if ($n -eq $c.lo) {
            $frName = $catFr[$c.id]
            [void]$exRows.AppendLine(
                "<h2 class=`"ex-cat`" id=`"cat-$n`"><span class=`"len`">$($c.en)</span><span class=`"lfr`">$frName</span></h2>")
        }
    }

    [void]$exRows.AppendLine("<details class=`"ex`" id=`"$name`">")
    [void]$exRows.AppendLine(
        "<summary><code>$name</code> - <span class=`"len`">$(Html $descEn)</span><span class=`"lfr`">$(Html $descFrt)</span></summary>")
    [void]$exRows.AppendLine('<div class="exbody">')
    $meta = "<a href=`"$gh/$name`">testsuite/$name</a> : " + ($srcLinks -join ' ')
    if ($ioNames) {
        $meta += " &middot; <span class=`"len`">inputs</span><span class=`"lfr`">entr&eacute;es</span>: " + ($ioNames -join ' ')
    }
    [void]$exRows.AppendLine("<p class=`"exmeta`">$meta</p>")

    foreach ($f in $auraFiles) {
        $rel = $f.FullName.Substring($dir.FullName.Length + 1) -replace '\\', '/'
        $code = Html ([IO.File]::ReadAllText($f.FullName))
        if ($auraFiles.Count -gt 1) {
            [void]$exRows.AppendLine("<p class=`"io`"><b>$rel</b></p>")
        }
        [void]$exRows.AppendLine("<pre><code class=`"aura`">$code</code></pre>")
    }
    foreach ($f in $inputs) {
        $rel = $f.FullName.Substring($dir.FullName.Length + 1) -replace '\\', '/'
        $lines = [IO.File]::ReadAllLines($f.FullName)
        if ($lines.Count -le 25 -and $f.Length -lt 4096) {
            $code = Html ($lines -join "`n")
            [void]$exRows.AppendLine("<p class=`"io`"><b>$rel</b></p><pre><code>$code</code></pre>")
        }
    }
    [void]$exRows.AppendLine('</div></details>')
}

$examplesHtml = @"
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Examples - Aura</title>
<meta name="description" content="50 real Aura programs - all compiled and tested in CI.">
<meta property="og:title" content="Aura Examples">
<meta property="og:description" content="50 real Aura programs, generated from the CI-tested testsuite.">
<meta property="og:type" content="website">
<meta property="og:url" content="https://tchoungageslin-blip.github.io/aura/examples.html">
<meta property="og:image" content="https://tchoungageslin-blip.github.io/aura/social.png">
<meta name="twitter:card" content="summary">
<link rel="icon" type="image/png" href="favicon.png">
<link rel="stylesheet" href="style.css">
</head>
<body>
$nav
<article style="max-width:900px">
<h1><span class="len">50 examples</span><span class="lfr">50 exemples</span></h1>
<p><span class="len">Every program below is real: it compiles, runs and
produces the expected output in CI - this page is generated from the
same <code>testsuite/</code> files the differential suite executes.
Click a scenario to unfold its source.</span><span class="lfr">Chaque programme ci-dessous est r&eacute;el : il compile, s'ex&eacute;cute et
produit la sortie attendue en CI - cette page est g&eacute;n&eacute;r&eacute;e depuis les
m&ecirc;mes fichiers <code>testsuite/</code> que la suite diff&eacute;rentielle ex&eacute;cute.
Clique sur un sc&eacute;nario pour d&eacute;plier sa source.</span></p>
<p><input id="exfilter" type="search" data-ph-en="Filter examples&hellip;" data-ph-fr="Filtrer les exemples&hellip;" oninput="
  var q = this.value.toLowerCase();
  document.querySelectorAll('details.ex').forEach(function (d) {
    d.style.display = !q || d.textContent.toLowerCase().indexOf(q) !== -1 ? '' : 'none';
  });
  document.querySelectorAll('h2.ex-cat').forEach(function (h) {
    var n = h.nextElementSibling, any = false;
    while (n && n.tagName === 'DETAILS') { if (n.style.display !== 'none') any = true; n = n.nextElementSibling; }
    h.style.display = any ? '' : 'none';
  });
"></p>
$exRows
</article>
<footer><p><span class="len">Generated from <code>testsuite/</code> - exercised by <code>aura test</code> on every commit.</span><span class="lfr">G&eacute;n&eacute;r&eacute; depuis <code>testsuite/</code> - exerc&eacute; par <code>aura test</code> &agrave; chaque commit.</span></p></footer>
<script src="i18n.js"></script>
<script src="aura.js"></script>
</body>
</html>
"@
Set-Content "$out/examples.html" $examplesHtml -Encoding UTF8

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
<p>Or read <a href="https://github.com/tchoungageslin-blip/aura/tree/master/docs/src">docs/src on GitHub</a>.</p>
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
