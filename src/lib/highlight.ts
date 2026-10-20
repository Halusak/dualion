// Syntax highlighting for the viewer and Quick View (highlight.js, a curated
// set of languages so the bundle stays small).

import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import c from 'highlight.js/lib/languages/c';
import cpp from 'highlight.js/lib/languages/cpp';
import csharp from 'highlight.js/lib/languages/csharp';
import css from 'highlight.js/lib/languages/css';
import dart from 'highlight.js/lib/languages/dart';
import diff from 'highlight.js/lib/languages/diff';
import dockerfile from 'highlight.js/lib/languages/dockerfile';
import go from 'highlight.js/lib/languages/go';
import gradle from 'highlight.js/lib/languages/gradle';
import ini from 'highlight.js/lib/languages/ini';
import java from 'highlight.js/lib/languages/java';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import kotlin from 'highlight.js/lib/languages/kotlin';
import less from 'highlight.js/lib/languages/less';
import lua from 'highlight.js/lib/languages/lua';
import makefile from 'highlight.js/lib/languages/makefile';
import markdown from 'highlight.js/lib/languages/markdown';
import objectivec from 'highlight.js/lib/languages/objectivec';
import perl from 'highlight.js/lib/languages/perl';
import php from 'highlight.js/lib/languages/php';
import powershell from 'highlight.js/lib/languages/powershell';
import python from 'highlight.js/lib/languages/python';
import r from 'highlight.js/lib/languages/r';
import ruby from 'highlight.js/lib/languages/ruby';
import rust from 'highlight.js/lib/languages/rust';
import scss from 'highlight.js/lib/languages/scss';
import sql from 'highlight.js/lib/languages/sql';
import swift from 'highlight.js/lib/languages/swift';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';
import scala from 'highlight.js/lib/languages/scala';
import groovy from 'highlight.js/lib/languages/groovy';
import vbnet from 'highlight.js/lib/languages/vbnet';
import dos from 'highlight.js/lib/languages/dos';
import nginx from 'highlight.js/lib/languages/nginx';
import graphql from 'highlight.js/lib/languages/graphql';
import haskell from 'highlight.js/lib/languages/haskell';
import elixir from 'highlight.js/lib/languages/elixir';

const LANGS = {
  bash, c, cpp, csharp, css, dart, diff, dockerfile, go, gradle, ini, java, javascript, json, kotlin, less, lua, makefile, markdown,
  objectivec, perl, php, powershell, python, r, ruby, rust, scss, sql, swift, typescript, xml, yaml, scala, groovy, vbnet, dos, nginx,
  graphql, haskell, elixir,
};
for (const [name, lang] of Object.entries(LANGS)) hljs.registerLanguage(name, lang);

const BY_EXT: Record<string, string> = {
  sh: 'bash', bash: 'bash', zsh: 'bash', fish: 'bash', command: 'bash', env: 'bash',
  c: 'c', h: 'c',
  cc: 'cpp', cpp: 'cpp', cxx: 'cpp', hpp: 'cpp', hh: 'cpp', hxx: 'cpp', ino: 'cpp',
  cs: 'csharp', csx: 'csharp',
  css: 'css', scss: 'scss', sass: 'scss', less: 'less',
  dart: 'dart',
  diff: 'diff', patch: 'diff',
  dockerfile: 'dockerfile',
  go: 'go',
  gradle: 'gradle', groovy: 'groovy', gvy: 'groovy',
  ini: 'ini', cfg: 'ini', conf: 'ini', toml: 'ini', properties: 'ini', editorconfig: 'ini', gitconfig: 'ini',
  java: 'java',
  js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'javascript',
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript',
  json: 'json', jsonc: 'json', json5: 'json', webmanifest: 'json', arb: 'json', geojson: 'json', lock: 'json',
  kt: 'kotlin', kts: 'kotlin',
  lua: 'lua',
  mk: 'makefile', makefile: 'makefile',
  md: 'markdown', markdown: 'markdown', mdx: 'markdown',
  m: 'objectivec', mm: 'objectivec',
  pl: 'perl', pm: 'perl',
  php: 'php', phtml: 'php',
  ps1: 'powershell', psm1: 'powershell', psd1: 'powershell',
  py: 'python', pyw: 'python', pyi: 'python',
  r: 'r',
  rb: 'ruby', gemspec: 'ruby', rake: 'ruby',
  rs: 'rust',
  sql: 'sql',
  swift: 'swift',
  html: 'xml', htm: 'xml', xhtml: 'xml', xml: 'xml', svg: 'xml', plist: 'xml', xaml: 'xml', csproj: 'xml', vue: 'xml', svelte: 'xml', storyboard: 'xml', xib: 'xml',
  yaml: 'yaml', yml: 'yaml',
  scala: 'scala', sc: 'scala',
  vb: 'vbnet',
  bat: 'dos', cmd: 'dos',
  graphql: 'graphql', gql: 'graphql',
  hs: 'haskell',
  ex: 'elixir', exs: 'elixir',
};

const BY_NAME: Record<string, string> = {
  dockerfile: 'dockerfile', makefile: 'makefile', gnumakefile: 'makefile', 'nginx.conf': 'nginx', gemfile: 'ruby', rakefile: 'ruby',
  podfile: 'ruby', '.bashrc': 'bash', '.zshrc': 'bash', '.profile': 'bash', 'cargo.lock': 'ini', 'pubspec.lock': 'yaml',
};

/** highlight.js language for a file name, or null for plain text. */
export function languageOf(name: string): string | null {
  const n = name.toLowerCase();
  if (BY_NAME[n]) return BY_NAME[n];
  const i = n.lastIndexOf('.');
  return i >= 0 ? (BY_EXT[n.slice(i + 1)] ?? null) : null;
}

/** Highlighted HTML (escaped) or null when the language is unknown or the text too big. */
export function highlight(text: string, lang: string | null, limit = 1_500_000): string | null {
  if (!lang || text.length > limit) return null;
  try {
    return hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
  } catch {
    return null;
  }
}
