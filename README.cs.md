# Dualion

**Česky** · [English](README.md)

Rychlý dvoupanelový správce souborů pro **Windows**, **macOS** a **Linux** ve stylu Total Commanderu – s nativním vzhledem každého systému, frontou operací, okamžitým hledáním nad indexem souborů, náhledy desítek formátů, archivy včetně hesel, vzdálenými úložišti (FTP, SFTP, WebDAV, S3, Android, Docker, rclone, MTP) se šifrovaným trezorem a českým i anglickým rozhraním.

Autor: **Haluška** · Komunita: [Discord](https://discord.com/invite/DjnKfuYYgN)

![Dualion – dva panely s kartami, lištou disků a metadatovými sloupci](docs/screenshots/main.png)

## Obsah

- [Panely a navigace](#panely-a-navigace)
- [Zobrazení a sloupce](#zobrazení-a-sloupce)
- [Operace se soubory](#operace-se-soubory)
- [Zpět, historie a simulace](#zpět-historie-a-simulace)
- [Hledání](#hledání)
- [Náhledy a prohlížeč F3](#náhledy-a-prohlížeč-f3)
- [Archivy a obrazy disků](#archivy-a-obrazy-disků)
- [Vzdálená úložiště a trezor hesel](#vzdálená-úložiště-a-trezor-hesel)
- [Nástroje](#nástroje)
- [Cloudové složky](#cloudové-složky-onedrive-icloud-dropbox)
- [Vzhled, jazyk a nastavení](#vzhled-jazyk-a-nastavení)
- [Klávesové zkratky](#klávesové-zkratky-výchozí)
- [Sestavení, testy, architektura](#sestavení)
- [Licence](#licence)

## Funkce

### Panely a navigace

- **Dva panely s kartami.** Nová karta, duplikace, zavření ostatních, přetažením lze karty přeřadit (zamčené i odemčené vždy v rámci své skupiny), prostředním tlačítkem zavřít. Lišta karet sedí těsně u okrajů panelu, aktivní karta aktivního panelu má modrou linku.
- **Zamčené karty** zůstávají vždy vlevo a nejdou zavřít; procházet se v nich dá normálně.
- **Karty zamčené se složkou:** uvnitř se dá procházet kamkoli, a když přepnete na jinou kartu a vrátíte se, karta znovu ukáže svou složku.
- **Přejmenování karet** dvojklikem na kartu nebo pravým klikem → Přejmenovat kartu.
- **Lišta disků** nad každým panelem (lze vypnout): disky s nativními ikonami, aktuální disk zvýrazněný, pamatuje si poslední složku na každém disku; prostřední tlačítko otevře disk v nové kartě. Tlačítko **Síť** nabídne počítače v síti, uložené servery, zařízení Android, telefony (MTP), kontejnery Docker, úložiště rclone a síťové svazky.
- **Místa v síti** (jako Síť v TC): počítače v místní síti → jejich sdílené složky → připojení a procházení. Windows ukáže počítače jako Průzkumník a o přihlášení se zeptá systémovým oknem; macOS najde SMB servery přes Bonjour a připojí je stejnými dialogy jako Finder.
- **Drobečková navigace**: klik na úroveň vrátí do složky, šipka mezi úrovněmi nabídne podsložky, klik za poslední položku přepne do psaní cesty s našeptávačem (`~`, `$HOME`, `%USERPROFILE%`, relativní cesty; cesta k souboru otevře jeho složku a vybere ho).
- Místa (Domov, Plocha, Dokumenty, Stažené…), **oblíbené složky** (hotlist), historie zpět/vpřed, položka „..“, skryté soubory, „Tento počítač“ / „Disky“ s volným místem.
- **Rychlé hledání psaním** (skok nebo filtrování) a **rychlý filtr** panelu (maska `*.jpg` i dotazovací jazyk).
- **Panel rychlého náhledu** (Quick View) v protějším panelu.
- Živé obnovování složek při změnách na disku, obnovení relace (karty, složky, zobrazení) po spuštění.
- Označování jako v TC: Insert / mezerník (označí a posune se dál, podržený mezerník značí postupně), Num + / − / * (maska, obrácení), stejná přípona, označení se klikem neruší; Ctrl/⌘+klik a Shift+klik jako ve Finderu / Průzkumníku.
- Drag & drop mezi panely, z Finderu a Průzkumníka do Dualionu i z Dualionu ven do Finderu, Průzkumníka nebo jiné aplikace.

![Světlý motiv – sloupce s metadaty fotek a procházení archivu .tar.gz](docs/screenshots/light.png)

### Zobrazení a sloupce

- Režimy: **podrobnosti**, **seznam** (sloupcový tok jako „Stručný“ v TC), **velké ikony**, **náhledy** (velikost miniatur nastavitelná).
- Nativní ikony souborů (stejné jako Finder / Průzkumník) a miniatury obrázků, videí, PDF, dokumentů, PSD, Affinity, RAW, obalů alb…
- **Sloupce** lze roztahovat úchytem na pravé hraně (i Název), dvojklik vrátí výchozí šířku; šířky si aplikace pamatuje zvlášť pro levý a pravý panel, nebo společně (volba v Nastavení).
- **Sloupce s metadaty**: rozměry, datum pořízení, fotoaparát, objektiv, ISO, clona, expozice, ohnisko, GPS, délka, kodek, FPS, datový tok, interpret, album, skladba, rok, žánr, vzorkování, počet stran, autor, název dokumentu, počet řádků. Volí se **automaticky podle obsahu složky** (fotky / video / hudba / dokumenty / kód), pravým klikem na záhlaví si je upravíte pro konkrétní složku; lze podle nich řadit.
- **Skupiny souborů**: `ikona.png` + `ikona.psd` + `ikona.ico`, `IMG_1.CR2` + `IMG_1.JPG` + `IMG_1.CR2.xmp`, `film.mkv` + `film.cs.srt`, `foto.jpg.json` se zobrazí jako jedna položka (+N, lze rozbalit) a kopírují, přesouvají, přejmenovávají a mažou se společně.
- **Zalamování názvů** souborů na 1–3 řádky (výchozí 1 řádek).
- Řazení podle názvu (přirozené, s češtinou), přípony, velikosti, data i metadat; složky vždy nahoře.
- Virtualizované seznamy – i složky se statisíci soubory jsou plynulé.

### Operace se soubory

- Kopírování, přesun, mazání (do koše nebo trvale), přejmenování přímo v řádku, nová složka (i vnořená `a/b/c`), nový soubor, duplikace (Shift+F5), symbolický odkaz.
- **Schránka**: vyjmout / kopírovat / vložit soubory, kopírování cesty k souboru, cesty ke složce a názvu.
- **Fronty operací**: úlohu lze spustit hned, nebo zařadit do jedné ze tří front (běží postupně; v dialogu kopírování / přesunu klávesou F2 jako v TC); pauza, pokračování, zrušení, spuštění mimo frontu, přeřazení ve frontě, průběh s rychlostí a odhadem času.
- Řešení konfliktů (nahradit / přeskočit / ponechat oba / nahradit starší, „pro všechny“), opakování po chybě, přeskočení chybných souborů.
- Rychlé kopírování: APFS klonování na macOS (okamžité), `CopyFileExW` na Windows; volitelné ověření kontrolním součtem, zachování časů, omezení rychlosti, filtr masky (`*.jpg;*.png | thumb*`).
- **Hromadné přejmenování** s náhledem (`[N] [N2-5] [E] [C] [Y][M][D] [h][m][s] [P]`, hledat/nahradit vč. regexu, velikost písmen).
- **Nativní kontextové menu:** na Windows menu Průzkumníku se všemi rozšířeními (Odeslat do, 7-Zip, OneDrive…) a příkazy Dualionu v podmenu; na macOS menu ve stylu Finderu s „Otevřít v aplikaci“ se seznamem aplikací, které soubor umí otevřít.
- Otevřít, otevřít v…, zobrazit ve Finderu / Průzkumníku, vlastnosti, Quick Look (macOS), editor F4 a terminál F9 (nastavitelné).

### Zpět, historie a simulace

- **Univerzální Zpět** (⌘Z / Ctrl+Z) a **historie operací** (⌘⇧Z): kopírování, přesun, mazání do koše, balení, rozbalení, nové složky, nové soubory, přejmenování i změny provedené pravidly jde vrátit. Vytvořené položky se při vrácení přesunou do koše, přesunuté se vrátí zpět, smazané se obnoví z koše.
- **Simulace před operací**: dialog kopírování / přesunu ukáže počet souborů a složek, velikost, konflikty (novější v cíli, shodné) a zda je na cíli dost místa; potvrzení mazání ukáže souhrn a zda jde akci vrátit.

![Simulace kopírování](docs/screenshots/copy-preview.png)

### Hledání

- **Okamžité hledání** (⌘P / Ctrl+P) nad **indexem souborů** ve stylu Everything: statisíce souborů v paměti, výsledky během psaní za pár milisekund, bez ohledu na diakritiku. Index se ukládá na disk, změny sleduje živě (FSEvents / ReadDirectoryChangesW) a metadata médií doplňuje na pozadí. Indexované složky, vynechané složky, skryté soubory a metadata se nastavují v Nastavení. Výsledky: přejít na soubor, otevřít, vše do panelu, uložit jako **chytrou složku**; přepínač „Všude / V této složce“.
- **Dotazovací jazyk** ve filtru panelu, v rychlém hledání i v dialogu Hledat: `faktura ext:pdf modified:2025`, `type:video res>=4k duration>10m codec:hevc`, `type:image camera:canon taken:2024 -ext:heic`, `size:1g..4g`, `artist:"Daft Punk"`, `orient:portrait`, `gps:yes`, `content:"text"`, `(a | b)`, `NOT`, české aliasy (`typ:`, `velikost:`, `změněno:`, `délka:`, `rozlišení:` …).
- **Chytré složky**: uložený dotaz se chová jako složka v oblíbených a při otevření se znovu vyhodnotí.
- **Hledání větou a podle významu** (volitelné, plně lokální přes Ollama): `?najdi všechny .dart soubory`, `?videa větší než 1 GB z loňska`, `?faktura za doménu example.com`. Věta se převede na filtry (jazykovým modelem, např. `gemma3:4b`, nebo vestavěnými pravidly pro češtinu a angličtinu) a téma se hledá podle významu v obsahu dokumentů včetně PDF (embedding model, např. `embeddinggemma`); okno ukáže, jak dotazu porozumělo. Volitelně i popisy fotek z vizuálního modelu. Nic neodchází z počítače.
- **Paleta příkazů** (⌘⇧P / Ctrl+Shift+P) s fuzzy hledáním všech akcí.
- **Vyhledávání v souborech** (Alt+F7, engine z ripgrepu): maska nebo regulární výraz názvu, text v souborech (regex, rozlišení velikosti, celá slova, „neobsahuje“, kódování vč. automatického Windows‑1250), dotaz s metadaty, velikost, datum, hloubka, vynechané složky, `.gitignore`, skryté soubory, symlinky. Výsledky v **tabulce s řaditelnými a roztažitelnými sloupci** (název s nalezeným řádkem, složka, přípona, detail – rozměry / délka / strany, velikost, datum, počet shod) a se **souhrnem podle typů**, který zároveň filtruje; výsledky lze zobrazit v panelu.

![Okamžité hledání nad indexem](docs/screenshots/quick-find.png)

![Vyhledávání v souborech – tabulka výsledků se souhrnem podle typů](docs/screenshots/search.png)

### Náhledy a prohlížeč F3

- **Prohlížeč F3** a **rychlý náhled**:
  - text s detekcí kódování (UTF‑8/16, Windows‑1250, ISO‑8859‑2, CP852, KOI8‑R, Shift_JIS, GBK…), zalamování, hledání (F3 / Shift+F3 další / předchozí výskyt), načítání velkých souborů po částech, hex režim,
  - **barevné zvýraznění syntaxe** (40 jazyků: Dart, JSON, HTML/XML, PHP, CSS/SCSS/Less, Rust, JavaScript/TypeScript, Python, Java, Kotlin, Swift, Go, C/C++, C#, SQL, Shell, PowerShell, YAML, TOML/INI, Markdown, Dockerfile, Makefile, Gradle, Ruby, Lua, Perl, R, Scala, Haskell, Elixir, GraphQL…) s čísly řádků,
  - obrázky (přiblížení), video, audio s obalem alba, PDF,
  - **tabulky** (`.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`, `.csv`, `.tsv`) s listy a záhlavím,
  - **dokumenty** (`.doc`, `.docx`, `.odt`, `.rtf`) s nadpisy, odrážkami a tabulkami,
  - **prezentace** (`.pptx`, `.odp`) se snímky,
  - **vzorník písem** (`.ttf`, `.otf`, `.woff`, `.woff2`),
  - vykreslení **PSD/PSB** (bez Photoshopu), **Affinity** (`.af`, `.afphoto`, `.afdesign`, `.afpub`), **RAW** fotek, **CorelDRAW** (`.cdr`, staré i nové verze), Clip Studio, Krita, OpenRaster, Sketch, Procreate, XMind, 3MF, iWork, EPUB.
- Miniatury: macOS ImageIO + QuickLook, Windows shell a vlastní dekodéry; mezipaměť v paměti i na disku.

![Prohlížeč F3 se zvýrazněním syntaxe](docs/screenshots/viewer-code.png)

![Náhled tabulky](docs/screenshots/preview-sheet.png)

### Archivy a obrazy disků

- **Procházení jako složka, náhled a rozbalení**: ZIP (i `.jar`, `.apk`, `.epub`, `.cbz`…), **7z**, **RAR** (RAR4 i RAR5), **ISO** (ISO 9660 / Joliet), **TAR**, `.tar.gz` / `.tgz`, `.tar.bz2`, `.tar.xz`, `.tar.zst` a samostatné `.gz`, `.bz2`, `.xz`, `.zst`.
- **Archivy s heslem**: ZIP (AES i ZipCrypto), 7z (AES‑256 vč. šifrovaných názvů), RAR – heslo se vyžádá při otevření či rozbalení, drží se jen v paměti.
- **Balení** do ZIP a 7z (volitelně s heslem AES‑256, u 7z i šifrované názvy) a do TAR / `.tar.gz` / `.tar.bz2` / `.tar.xz` / `.tar.zst`, s volbou komprese.
- **Připojení obrazů disků** (ISO, DMG, VHD…) systémem – `hdiutil` na macOS, `Mount-DiskImage` na Windows.

### Vzdálená úložiště a trezor hesel

Každé vzdálené úložiště se v panelu otevře jako složka. Kopírování (i přímo mezi dvěma vzdálenými úložišti), přesun, přejmenování, mazání, fronty, ověřování, náhledy i otevírání souborů fungují všude stejně.

- **FTP / FTPS:** explicitní i implicitní TLS, pasivní / aktivní režim, kódování názvů vč. Windows‑1250.
- **SFTP (SSH):** přihlášení souborem s klíčem (i s heslem ke klíči), přes SSH agenta (Pageant / agent OpenSSH ve Windows), klíči v `~/.ssh` nebo heslem; klíče serverů se ověřují proti `~/.ssh/known_hosts` – neznámý klíč ukáže otisk k potvrzení, změněný klíč je odmítnut.
- **WebDAV / WebDAVS:** Nextcloud, ownCloud, Synology, SharePoint, Apache, nginx… Server jde zadat i celou adresou URL.
- **S3 a kompatibilní úložiště:** AWS, Cloudflare R2, Backblaze B2, Wasabi, MinIO… V kořeni buckety, složky jako prefixy, velké soubory se nahrávají po částech.
- **Automaticky nalezené** (tlačítko Síť): **zařízení Android** přes ADB (USB i Wi‑Fi ladění) s přesnými velikostmi, časy a právy; **telefony a fotoaparáty přes MTP** na Windows bez vývojářského režimu; **kontejnery Docker / Podman**; **úložiště rclone** – každá služba nastavená v rclone (Google Drive, OneDrive, Dropbox, Mega, pCloud, Box… přes 70).
- Uložené přístupy jsou v **trezoru šifrovaném XChaCha20‑Poly1305**; klíč je v Klíčence macOS / Správci pověření Windows, nebo odvozený z hlavního hesla (Argon2id). Hesla nikdy neopustí backend.

### Nástroje

- **Mapa využití disku** (treemap) s proklikem do složek, seznamem největších položek a mazáním do koše.
- **Duplicity**: shodné soubory podle obsahu (velikost → rychlý otisk → plný hash, s využitím indexu) i **podobné fotky** podle percepčního otisku; automatické označení všech kromě nejstaršího / nejnovějšího / nejkratší cesty / největšího, smazání do koše (vratné), výsledky do panelu.
- **Pravidla pro složky** (jako Hazel): např. „PDF faktury ze Stažených přesuň do `Dokumenty/Faktury/{year}/{month}`“; podmínka v dotazovacím jazyce, akce přesunout / kopírovat / přejmenovat / do koše, šablony `{name} {ext} {year} {month} {day} {type} {camera} {artist} {album}`, náhled, spuštění ručně nebo automaticky na nové soubory, záznam aktivity, vše vratné.
- **Kontrolní součty** (MD5, SHA‑1, SHA‑256, SHA‑512, CRC32, XXH3), **porovnání složek**, **výpočet velikosti složek**, symbolické odkazy.

![Mapa využití disku](docs/screenshots/disk-usage.png)

![Duplicity](docs/screenshots/duplicates.png)

![Pravidla pro složky](docs/screenshots/rules.png)

### Cloudové složky (OneDrive, iCloud, Dropbox…)

- Soubory „jen online“ se poznají bez stahování (Windows: atributy Files On‑Demand, macOS: příznak *dataless*) a jsou označené **obláčkem**.
- Index, miniatury, metadata, rychlý náhled, hledání v obsahu, duplicity ani sémantické hledání je **nikdy automaticky nestahují**; stáhnou se jen při výslovném otevření (nebo tlačítkem „Stáhnout a zobrazit náhled“).
- Cloudové složky (všechny účty OneDrive, Dropbox, Google Drive, iCloud) lze z indexu vynechat úplně.

### Vzhled, jazyk a nastavení

- **macOS**: sjednocený titulek s vibrancy, vycentrované akce, nativní menu a kontextová menu, systémová barva zvýraznění.
- **Windows 11**: Mica, Fluent styl, lišta menu v okně (otevírání přejetím, Alt, Alt+písmeno, šipky), pořadí tlačítek dialogů dle Windows, bez okna konzole.
- Světlý / tmavý / systémový motiv, hustota řádků, velikost písma, formát velikostí (automaticky / binární / desítkový / bajty), formát data a času (podle jazyka, předvolby jako `21.10.2026 18:48` nebo `2026-10-21 18:48`, nebo vlastní vzor).
- **Čeština a angličtina** (výchozí podle systému), české texty i v instalátorech pro Windows.
- **Vlastní klávesové zkratky** (Nastavení → Klávesové zkratky) – změny se propíší do menu i nápovědy F1.
- Nápověda F1 se seznamem zkratek, O aplikaci (Discord, podpora vývoje).
- Každou z rozšířených funkcí (lišta disků, metadatové sloupce, skupiny souborů, index, pravidla, sémantické hledání) lze v Nastavení vypnout.

## Klávesové zkratky (výchozí)

| Klávesa | Akce | Klávesa | Akce |
|---|---|---|---|
| F1 | Nápověda / zkratky | F7 | Nová složka |
| F2, Shift+F6 | Přejmenovat | F8 | Smazat (do koše) |
| F3 | Zobrazit | Shift+F8 | Smazat trvale |
| F4 | Upravit | F9 | Terminál |
| Shift+F4 | Nový soubor | F10 | Konec |
| F5 | Kopírovat | Alt+F7, Mod+F | Hledat v souborech |
| Shift+F5 | Duplikovat | Alt+F1 / Alt+F2 | Disk vlevo / vpravo |
| F6 | Přesunout | Alt+F5 / Alt+F9 | Zabalit / rozbalit |
| Tab | Přepnout panel | Mod+T / Mod+W | Nová / zavřít kartu |
| Insert, Mezerník | Označit | Mod+Shift+L | Zamknout kartu |
| Num + / − / * | Výběr maskou / obrácení | Mod+K | Připojit k serveru |
| Mod+L | Psát cestu | Mod+M | Hromadné přejmenování |
| Mod+P | Okamžité hledání | Mod+Shift+P | Paleta příkazů |
| Mod+Z | Zpět (poslední operace) | Mod+Shift+Z | Historie operací |
| Mod+Shift+C | Kopírovat cestu k souboru | Mod+Alt+Shift+C | Kopírovat cestu ke složce |
| Mod+Alt+C | Kopírovat název | Mod+Alt+G | Seskupovat soubory |
| Mod+Alt+U | Mapa disku | Mod+Alt+D | Duplicity |
| Mod+1 … 4 | Podrobnosti / seznam / ikony / náhledy | Mod+Alt+L | Symbolický odkaz |
| F12, Ctrl+Q | Rychlý náhled | Mod+Shift+G | Přejít na cestu ze schránky |

`Mod` = ⌘ na macOS, Ctrl na Windows. Kompletní seznam je v aplikaci pod F1 a všechny zkratky lze změnit v Nastavení.

## Instalace na Linuxu

Ke stažení v [Releases](https://github.com/Halusak/dualion/releases):

- **Debian, Ubuntu, Mint…:** `sudo apt install ./Dualion_<verze>_amd64.deb`
- **Arch Linux:** stáhněte `PKGBUILD` do prázdné složky a spusťte `makepkg -si`
- **Jakákoli distribuce:** AppImage – `chmod +x Dualion_<verze>_amd64.AppImage` a spustit (potřebuje FUSE 2; na Archu `sudo pacman -S fuse2`)

Volitelné nástroje zapnou další funkce: `udisks2` (obrazy disků), `gvfs-smb` / `gvfs-backends`, `smbclient` a `avahi-utils` (síťová sdílení), `adb`, `docker`, `rclone`. Na Linuxu zatím chybí přetahování souborů ven z okna a zařízení MTP (Android jde přes ADB).

## Sestavení

Potřebujete [Rust](https://rustup.rs) (stable) a Node.js ≥ 20.19.

- **macOS**: Xcode Command Line Tools (`xcode-select --install`).
- **Windows**: Visual Studio Build Tools s „Desktop development with C++“ (kvůli UnRAR a zstd); WebView2 je ve Windows 10/11 předinstalovaný (instalátor ho případně doinstaluje).

```bash
npm ci
npm run tauri dev      # vývoj s hot reloadem
npm run tauri build    # instalátory
```

Výstupy `npm run tauri build`:

- macOS: `src-tauri/target/release/bundle/macos/Dualion.app` a `bundle/dmg/Dualion_<verze>_<arch>.dmg`
- Windows: `src-tauri/target/release/bundle/msi/Dualion_<verze>_x64_cs-CZ.msi` a `bundle/nsis/Dualion_<verze>_x64-setup.exe`
- Linux: `src-tauri/target/release/bundle/deb/*.deb` a `bundle/appimage/*.AppImage` (potřebuje `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`, `libxdo-dev`, `libssl-dev`)

Univerzální build pro Mac (Intel + Apple Silicon): `npm run tauri build -- --target universal-apple-darwin`.
Instalátory pro obě platformy sestaví i GitHub Actions (`.github/workflows/release.yml`) po pushnutí tagu `v*`.

Pro distribuci mimo vlastní počítač je vhodné aplikaci podepsat (macOS: Developer ID + notarizace, Windows: Authenticode); proměnné pro podepisování jsou popsané v `release.yml`.

Volitelné AI funkce potřebují [Ollama](https://ollama.com) s modely, které lze stáhnout přímo z Nastavení (např. `embeddinggemma` a `gemma3:4b`).

## Testy

```bash
npm test                      # frontend (Vitest): zkratky, skupiny, treemap, zvýraznění syntaxe…
npm run check                 # typová kontrola (svelte-check)
cd src-tauri && cargo test    # backend: kopírování, fronty, konflikty, archivy, index, dotazy, metadata, trezor, hledání…
```

Volitelné testy proti externím zdrojům (`cargo test … -- --ignored`):

```bash
DUALION_FTP=127.0.0.1:2121 DUALION_FTPS=127.0.0.1:2122 cargo test ftp_e2e -- --ignored   # FTP (uživatel jan, heslo tajne)
DUALION_SFTP=127.0.0.1:2222 DUALION_SFTP_KEY=… cargo test sftp_e2e -- --ignored        # SFTP (a WebDAV, S3, ADB, Docker, rclone:
                                                                                        #  viz src-tauri/src/jobs/remote_tests.rs)
DUALION_RAR_DATA=<unrar-crate>/data cargo test rar -- --ignored                          # RAR včetně hesla
cargo test ollama -- --ignored                                                           # embeddingy přes Ollamu
```

## Architektura

- `src-tauri/` – Rust backend (Tauri 2):
  - výpisy a svazky (`local.rs`), fronta úloh a kopírovací engine (`jobs/`), vracení akcí a simulace (`jobs/undo.rs`, `jobs/plan.rs`),
  - archivy (`arch/` – ZIP, 7z, RAR, ISO, TAR*, balení), vzdálená úložiště (`remote/` – FTP, SFTP, WebDAV, S3, ADB, Docker, rclone, MTP), šifrovaný trezor (`vault.rs`),
  - vyhledávání (`search.rs`), index souborů (`index/`), dotazovací jazyk (`query.rs`), věty → dotazy (`ask.rs`), sémantické hledání (`semantic.rs`),
  - nativní ikony a miniatury (`icons/`, `thumbs.rs`), vlastní dekodéry náhledů (`preview/`), metadata (`meta/`), prohlížeč (`viewer.rs`),
  - mapa disku (`diskusage.rs`), duplicity (`dupes.rs`), pravidla (`rules.rs`), štítky (`tags.rs`), nativní menu (`menu.rs`), sledování změn složek (`watcher.rs`), platformní funkce (`platform.rs`).
- `src/` – Svelte 5 + TypeScript: stav panelů, karet, indexu a historie (`lib/state/`), příkazy a klávesové zkratky (`lib/commands.ts`), akce (`lib/actions.ts`), sloupce a skupiny (`lib/columns.ts`, `lib/groups.ts`), zvýraznění syntaxe (`lib/highlight.ts`), komponenty (`components/`), překlady (`lib/i18n/`).

Nastavení, relace, index a trezor se ukládají do systémové složky aplikace (`~/Library/Application Support/cz.dualion.commander`, resp. `%APPDATA%\cz.dualion.commander`).

## Licence

Copyright © 2026 Haluška. Dualion je svobodný software pod licencí [GNU General Public License v3.0](LICENSE).

Program smíte používat, studovat, upravovat a šířit. Pokud šíříte upravenou verzi, musíte zveřejnit i její zdrojový kód pod stejnou licencí.

## Podpora vývoje

Dualion je zdarma. Pokud vám pomáhá, můžete přispět:

- [Revolut](https://revolut.me/lukas399?note=Dualion)
- [PayPal](https://www.paypal.com/donate/?hosted_button_id=9QY42AUNSBRWE)
- BTC: `bc1qvfxwdqk9qp02tnnzvxtcd8sq5z7umud07xul33`
- ETH: `0x9B14c9f5c8A7891e43ff318DeAC7da1C213a4BAC`
