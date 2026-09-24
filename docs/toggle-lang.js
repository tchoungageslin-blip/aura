// Adds an EN|FR switch to mdBook pages that have a translated mirror,
// and honours the site-level choice stored in localStorage (auraLang).
(function () {
  var map = {
    'getting-started.html': 'fr/demarrage.html',
    'cheatsheet.html': 'fr/cheatsheet.html',
    'tutorials/index.html': 'tutorials/fr/index.html',
    'tutorials/cli-tool.html': 'tutorials/fr/cli-tool.html',
    'tutorials/file-processing.html': 'tutorials/fr/file-processing.html',
    'tutorials/kv-store.html': 'tutorials/fr/kv-store.html',
    'tutorials/project.html': 'tutorials/fr/project.html'
  };
  var rev = {};
  Object.keys(map).forEach(function (k) { rev[map[k]] = k; });

  // path_to_root is injected by mdBook ("", "../", "../../", ...).
  var root = typeof path_to_root !== 'undefined' ? path_to_root : '';
  var depth = (root.match(/\.\.\//g) || []).length;
  var parts = window.location.pathname.split('/').filter(Boolean);
  var rel = parts.slice(parts.length - depth - 1).join('/');

  var target = map[rel];
  var label = 'FR';
  if (!target && rev[rel]) { target = rev[rel]; label = 'EN'; }
  if (!target) return;

  // Follow the site preference once: if the visitor picked FR on the
  // static pages and this page has a French mirror, switch to it.
  try {
    var pref = localStorage.getItem('auraLang');
    if (pref === 'fr' && map[rel]) { window.location.replace(root + map[rel]); return; }
    if (pref === 'en' && rev[rel]) { window.location.replace(root + rev[rel]); return; }
  } catch (e) {}

  var bar = document.querySelector('.right-buttons');
  if (!bar) return;
  var a = document.createElement('a');
  a.href = root + target;
  a.textContent = label;
  a.title = label === 'FR' ? 'Version française' : 'English version';
  a.style.cssText = 'margin-left:10px;padding:2px 10px;border:1px solid var(--sidebar-bg);border-radius:6px;font-size:13px;';
  a.addEventListener('click', function () {
    try { localStorage.setItem('auraLang', label.toLowerCase()); } catch (e) {}
  });
  bar.appendChild(a);
})();
