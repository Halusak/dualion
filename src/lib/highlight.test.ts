import { describe, expect, it } from 'vitest';
import { highlight, languageOf } from './highlight';

describe('syntax highlighting', () => {
  const samples: [string, string, string][] = [
    ['main.dart', "void main() { final name = 'Dart'; print(name); }", 'hljs-keyword'],
    ['lib.rs', 'fn main() { let x: u32 = 42; println!("{}", x); }', 'hljs-keyword'],
    ['app.js', "const a = () => 'x'; // hi", 'hljs-comment'],
    ['data.json', '{"a": 1, "b": true}', 'hljs-attr'],
    ['index.html', '<div class="x">Hi</div>', 'hljs-tag'],
    ['page.php', '<?php echo $name; ?>', 'hljs-variable'],
    ['style.css', '.btn { color: red; }', 'hljs-selector-class'],
    ['Dockerfile', 'FROM node:20\nRUN npm ci', 'hljs-keyword'],
    ['config.toml', '[package]\nname = "x"', 'hljs-section'],
  ];
  for (const [name, code, cls] of samples) {
    it(name, () => {
      const lang = languageOf(name);
      expect(lang).not.toBeNull();
      const html = highlight(code, lang);
      expect(html).toContain(cls);
    });
  }
  it('escapes HTML and skips unknown types', () => {
    expect(highlight('<script>', languageOf('a.js'))).not.toContain('<script>');
    expect(languageOf('notes.xyz')).toBeNull();
    expect(highlight('x'.repeat(20), 'javascript', 10)).toBeNull();
  });
});
