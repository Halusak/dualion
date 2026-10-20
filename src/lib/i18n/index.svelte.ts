import { cs } from './cs';
import { en, type Key } from './en';

export type Lang = 'cs' | 'en';

const dicts: Record<Lang, Record<string, string>> = { cs, en };

export const i18n = $state<{ lang: Lang }>({ lang: 'en' });

export function langFromLocale(locale: string | null | undefined): Lang {
  return (locale ?? '').toLowerCase().startsWith('cs') || (locale ?? '').toLowerCase().startsWith('sk') ? 'cs' : 'en';
}

/** BCP-47 tag used for number/date formatting. */
export function localeTag(): string {
  return i18n.lang === 'cs' ? 'cs-CZ' : 'en-US';
}

const pluralRules = new Map<Lang, Intl.PluralRules>();

type Params = Record<string, string | number>;

function lookup(lang: Lang, key: string): string | undefined {
  return dicts[lang][key] ?? dicts.en[key];
}

/**
 * Translates `key`, substituting `{param}` placeholders. Keys with a
 * `count` param resolve plural variants (`key_one`, `key_few`, `key_other`…).
 */
export function t(key: Key | (string & {}), params?: Params): string {
  const lang = i18n.lang;
  let s: string | undefined;
  if (params && typeof params.count === 'number') {
    let pr = pluralRules.get(lang);
    if (!pr) {
      pr = new Intl.PluralRules(lang);
      pluralRules.set(lang, pr);
    }
    const form = pr.select(params.count);
    s = dicts[lang][`${key}_${form}`] ?? dicts[lang][`${key}_other`] ?? dicts.en[`${key}_${form}`] ?? dicts.en[`${key}_other`];
  }
  s ??= lookup(lang, key) ?? key;
  if (params) {
    s = s.replace(/\{(\w+)\}/g, (m, p: string) => (p in params ? String(params[p]) : m));
  }
  return s;
}

/** "„report.pdf“" for one item, "3 položky" for more. */
export function what(names: string[]): string {
  if (names.length === 1) return t('what.one', { name: names[0] });
  return t('what.items', { count: names.length });
}
