# Dualion

Rychlý dvoupanelový správce souborů pro **Windows** a **macOS** ve stylu Total Commanderu – s nativním vzhledem každého systému, frontou operací, vyhledáváním v obsahu souborů, FTP/FTPS se šifrovaným trezorem hesel a českým i anglickým rozhraním.

Autor: **Haluška** · Komunita: [Discord](https://discord.com/invite/DjnKfuYYgN)

## Funkce

**Panely a navigace**
- Dva panely s kartami; karty lze **zamknout** (zůstávají vždy vlevo a jiná složka se z nich otevře v nové kartě, jako v TC).
- Drobečková navigace nad panelem: klik na libovolnou úroveň vrátí do dané složky, šipka mezi úrovněmi nabídne podsložky, klik za poslední položku přepne do psaní cesty s našeptávačem (`~`, `$HOME`, `%USERPROFILE%`, relativní cesty, cesta k souboru otevře jeho složku).
- Režimy zobrazení: podrobnosti (řaditelné a roztažitelné sloupce), seznam (sloupcový tok jako „Stručný“ v TC), velké ikony, náhledy.
- Nativní ikony souborů (stejné jako Finder / Průzkumník) a náhledy obrázků, videí, PDF a dokumentů (macOS ImageIO + QuickLook, Windows shell).
- Vlastní náhledy formátů, které systém často neumí: PSD/PSB (bez Photoshopu), Affinity (`.af`, `.afphoto`, `.afdesign`, `.afpub`), Krita, OpenRaster, Sketch, Procreate, Clip Studio, XMind, 3MF, RAW fotky, obaly alb v MP3/M4A/FLAC, Office (`.docx`, `.xlsx`, `.pptx`), OpenDocument, iWork a EPUB.
- Virtualizované seznamy – i složky se stovkami tisíc souborů jsou plynulé.
- Disky a místa (s volným místem), oblíbené složky, historie zpět/vpřed, skryté soubory, rychlý filtr, rychlé hledání psaním, panel rychlého náhledu.

**Operace se soubory**
- Kopírování, přesun, mazání (do koše nebo trvale), přejmenování přímo v řádku, nová složka (i vnořená `a/b/c`), nový soubor.
- **Fronty operací**: úlohu lze spustit hned, nebo zařadit do jedné z front (běží postupně); pauza, pokračování, zrušení, spuštění mimo frontu, přeřazení ve frontě, průběh s rychlostí a odhadem času.
- Řešení konfliktů (nahradit / přeskočit / ponechat oba / nahradit starší, „pro všechny“), opakování po chybě.
- Rychlé kopírování: APFS klonování na macOS (okamžité), `CopyFileExW` na Windows; volitelné ověření kontrolním součtem, omezení rychlosti, filtr masky (`*.jpg;*.png | thumb*`).
- Drag & drop mezi panely i z Finderu / Průzkumníka.

**Nástroje**
- **Vyhledávání** (engine z ripgrepu): maska nebo regulární výraz názvu, hledání textu v souborech (regex, celá slova, „neobsahuje“), velikost, datum, hloubka, vynechání složek, `.gitignore`. Texty ve Windows‑1250 a dalších kódováních najde i automaticky. Výsledky lze zobrazit v panelu.
- **FTP / FTPS** (explicitní i implicitní TLS, pasivní/aktivní režim, kódování názvů vč. Windows‑1250). Přístupy jsou uložené v trezoru šifrovaném XChaCha20‑Poly1305; klíč je v Klíčence macOS / Správci pověření Windows, nebo odvozený z hlavního hesla (Argon2id). Hesla nikdy neopustí backend.
- ZIP archivy: procházení jako složka, rozbalení, zabalení.
- Hromadné přejmenování s náhledem (`[N] [N2-5] [E] [C] [Y][M][D] [h][m][s] [P]`, hledat/nahradit vč. regexu, velikost písmen, vrácení akce).
- Prohlížeč F3 a panel rychlého náhledu: text s detekcí kódování, hex, obrázky, video, audio, PDF, tabulky (`.xls`, `.xlsx`, `.xlsb`, `.ods`, `.csv`/`.tsv` vč. listů), dokumenty (`.doc`, `.docx`, `.odt`, `.rtf`), prezentace (`.pptx`, `.odp`), vzorník písem (`.ttf`, `.otf`, `.woff`) a vykreslení PSD/Affinity/RAW; editor F4, terminál F9, kontrolní součty (MD5/SHA‑1/SHA‑256/SHA‑512/CRC32/XXH3), porovnání složek, výpočet velikosti složek, symbolické odkazy.

**Vzhled a ovládání**
- macOS: sjednocený titulek s vibrancy, vycentrované akce, nativní menu a kontextová menu, „sheety“, systémová barva zvýraznění.
- Windows 11: Mica, Fluent styl, nativní menu, pořadí tlačítek dialogů dle Windows.
- Světlý i tmavý motiv, čeština a angličtina (výchozí podle systému).
- **Vlastní klávesové zkratky** (Nastavení → Klávesové zkratky) – změny se propíší do menu i nápovědy F1.

## Klávesové zkratky (výchozí)

| Klávesa | Akce | Klávesa | Akce |
|---|---|---|---|
| F1 | Nápověda / zkratky | F7 | Nová složka |
| F2, Shift+F6 | Přejmenovat | F8 | Smazat (do koše) |
| F3 | Zobrazit | Shift+F8 | Smazat trvale |
| F4 | Upravit | F9 | Terminál |
| Shift+F4 | Nový soubor | F10 | Konec |
| F5 | Kopírovat | Alt+F7, Mod+F | Hledat |
| Shift+F5 | Duplikovat | Alt+F1 / Alt+F2 | Disk vlevo / vpravo |
| F6 | Přesunout | Alt+F5 / Alt+F9 | Zabalit / rozbalit |
| Tab | Přepnout panel | Mod+T / Mod+W | Nová / zavřít kartu |
| Insert, Mezerník | Označit | Mod+Shift+P | Zamknout kartu |
| Num + / − / * | Výběr maskou / obrácení | Mod+K | Připojit k serveru |
| Mod+L | Psát cestu | Mod+M | Hromadné přejmenování |

`Mod` = ⌘ na macOS, Ctrl na Windows. Kompletní seznam je v aplikaci pod F1.

## Sestavení

Potřebujete [Rust](https://rustup.rs) (stable) a Node.js ≥ 20.19.

- **macOS**: Xcode Command Line Tools (`xcode-select --install`).
- **Windows**: Visual Studio Build Tools s „Desktop development with C++“; WebView2 je ve Windows 10/11 předinstalovaný (instalátor ho případně doinstaluje).

```bash
npm ci
npm run tauri dev      # vývoj s hot reloadem
npm run tauri build    # instalátory
```

Výstupy `npm run tauri build`:

- macOS: `src-tauri/target/release/bundle/macos/Dualion.app` a `bundle/dmg/Dualion_<verze>_<arch>.dmg`
- Windows: `src-tauri/target/release/bundle/msi/Dualion_<verze>_x64_cs-CZ.msi` a `bundle/nsis/Dualion_<verze>_x64-setup.exe`

Univerzální build pro Mac (Intel + Apple Silicon): `npm run tauri build -- --target universal-apple-darwin`.
Instalátory pro obě platformy sestaví i GitHub Actions (`.github/workflows/release.yml`) po pushnutí tagu `v*`.

Pro distribuci mimo vlastní počítač je vhodné aplikaci podepsat (macOS: Developer ID + notarizace, Windows: Authenticode); proměnné pro podepisování jsou popsané v `release.yml`.

## Testy

```bash
npm test                      # frontend (Vitest)
npm run check                 # typová kontrola (svelte-check)
cd src-tauri && cargo test    # backend: kopírování, fronty, konflikty, ZIP, trezor, hledání…
```

FTP testy proti skutečnému serveru (uživatel `jan`, heslo `tajne`, právo zápisu):

```bash
DUALION_FTP=127.0.0.1:2121 DUALION_FTPS=127.0.0.1:2122 cargo test ftp_e2e -- --ignored
```

## Architektura

- `src-tauri/` – Rust backend (Tauri 2): výpisy a svazky (`local.rs`), fronta úloh a kopírovací engine (`jobs/`), FTP (`ftp.rs`), šifrovaný trezor (`vault.rs`), vyhledávání (`search.rs`), nativní ikony a náhledy (`icons/`, `thumbs.rs`), vlastní dekodéry náhledů (`preview/`), prohlížeč (`viewer.rs`), nativní menu (`menu.rs`), sledování změn složek (`watcher.rs`).
- `src/` – Svelte 5 + TypeScript: stav panelů a karet (`lib/state/`), příkazy a klávesové zkratky (`lib/commands.ts`), akce (`lib/actions.ts`), komponenty (`components/`), překlady (`lib/i18n/`).

Nastavení, relace a trezor se ukládají do systémové složky aplikace (`~/Library/Application Support/cz.dualion.commander`, resp. `%APPDATA%\cz.dualion.commander`).

## Podpora vývoje

Dualion je zdarma. Pokud vám pomáhá, můžete přispět:

- [Revolut](https://revolut.me/lukas399?note=Dualion)
- [PayPal](https://www.paypal.com/donate/?hosted_button_id=9QY42AUNSBRWE)
- BTC: `bc1qvfxwdqk9qp02tnnzvxtcd8sq5z7umud07xul33`
- ETH: `0x9B14c9f5c8A7891e43ff318DeAC7da1C213a4BAC`
