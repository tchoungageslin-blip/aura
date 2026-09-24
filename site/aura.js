// Tiny Aura highlighter — colors every <code class="aura"> block.
// Keep in sync with docs/src/language.md "Lexical structure".
(function () {
  // One combined regex, one pass — group order = priority.
  var RE = new RegExp([
    '(\\/\\/[^\\n]*|\\/\\*[\\s\\S]*?\\*\\/)',                       // 1 comment
    '("(?:[^"\\\\\\n]|\\\\.)*")',                                   // 2 string
    '\\b(0x[0-9a-fA-F_]+|0b[01_]+|0o[0-7_]+|\\d[\\d_]*(?:\\.\\d+)?(?:[eE][+-]?\\d+)?)\\b', // 3 number
    '\\b(fn|let|mut|if|else|while|loop|return|struct|enum|match|use|unsafe|extern|break|continue|true|false)\\b', // 4 keyword
    '\\b(print|println|eprint|eprintln|exit|sqrt|str_from_int|str_from_bool|str_get|str_slice|str_from_byte|str_from_f64|f64_from_int|vec_new|vec_push|vec_get|vec_set|vec_pop|args|env|read_file|write_file|read_stdin|exec|Ok|Err)\\b', // 5 builtin
    '\\b(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize|f32|f64|bool|str|vec|Result)\\b', // 6 type
  ].join('|'), 'g');
  var CLS = ['tok-com', 'tok-str', 'tok-num', 'tok-kw', 'tok-builtin', 'tok-type'];

  function highlight(html) {
    return html.replace(RE, function (m, c1, c2, c3, c4, c5, c6) {
      var groups = [c1, c2, c3, c4, c5, c6];
      for (var i = 0; i < groups.length; i++) {
        if (groups[i] !== undefined) {
          return '<span class="' + CLS[i] + '">' + m + '</span>';
        }
      }
      return m;
    });
  }

  document.querySelectorAll('code.aura').forEach(function (el) {
    el.innerHTML = highlight(el.innerHTML);
  });
})();
