// Language toggle — marks every .len / .lfr element visible per
// <html lang>, persists the choice in localStorage, and injects an
// EN|FR button into each page's nav.
(function () {
  var stored = null;
  try { stored = localStorage.getItem('auraLang'); } catch (e) {}
  var lang = stored === 'fr' ? 'fr' : 'en';

  function apply(l) {
    document.documentElement.lang = l;
    try { localStorage.setItem('auraLang', l); } catch (e) {}
    document.querySelectorAll('.langbtn').forEach(function (b) {
      b.textContent = l === 'en' ? 'FR' : 'EN';
      b.title = l === 'en' ? 'Passer en français' : 'Switch to English';
    });
  }

  window.AURA_I18N = {
    toggle: function () {
      apply(document.documentElement.lang === 'en' ? 'fr' : 'en');
    },
    lang: function () { return document.documentElement.lang; }
  };

  // Runs at end of <body>: nav markup is already present.
  document.querySelectorAll('nav > div').forEach(function (nav) {
    var b = document.createElement('button');
    b.type = 'button';
    b.className = 'langbtn';
    b.addEventListener('click', window.AURA_I18N.toggle);
    nav.appendChild(b);
  });
  apply(lang);
})();
