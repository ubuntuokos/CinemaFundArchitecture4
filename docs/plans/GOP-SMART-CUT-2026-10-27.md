# CFA4 GOP-alapú Smart Cut – megvalósítási terv

**Dátum:** 2026-10-27  
**Státusz:** a terv tulajdonos által jóváhagyva (2026-10-27); a natív CFA4-integráció irányát a korábbi tulajdonosi döntés rögzíti  
**Kapcsolódó terv:** [Smart Render](SMART-RENDER-2026-10-27.md)

## Cél

Egyetlen videóforrás képkocka-pontos In/Out tartományának kivágása úgy, hogy a teljes, biztonságosan másolható belső GOP-ok változatlan tömörített mintákként kerüljenek át, és csak a vágási határokhoz szükséges képkockák/GOP-ok legyenek újrakódolva. A GOPchop működési elve a CFA4 saját Rust-médiamotorjába épül; nincs külső folyamat és nincs külön alkalmazás.

## Kiinduló állapot

- `crates/codecs/src/gop.rs`: GOP-tudatos véletlen elérés, kulcsképkockáról induló dekódolás és képkocka-cache. Ez nem csomagolt minták vágója.
- `crates/isobmff`: MP4/MOV mintatáblák, mintánkénti olvasás és progresszív írás.
- `crates/matroska`: Matroska demux és mux.
- `crates/mpegts`: MPEG-TS/PS demux, PES/access-unit index, random-access jelzések; muxer még külön fejlesztési feladat.
- `crates/export`: képkocka-renderelésből kódol; nincs vegyes mintamásoló/határ-újrakódoló Smart Cut útvonal.
- `crates/engine`: a szerkesztési parancsok közös útja UI, CLI és control channel számára.

A vágási számítás CFA4 `Tick`-ben, a demux indexe pedig minta- és decode/display-sorrendet megőrizve működjön. A konténer képességéből, codec configból és GOP-szerkezetből kell eldönteni a biztonságos copy-tartományt. Nem elegendő a fájlkiterjesztés vagy egy általános keyframe flag.

## Garanciák és felhasználói módok

1. **Képkocka-pontos mód:** az eredmény a kiválasztott forrásképkocka-határokat tartja; a szükséges határ-GOP-ok dekódolódnak és újrakódolódnak.
2. **GOP-határ mód:** stream-copy vágás a biztonságos random-access határokon. A CFA4 a kiválasztott és tényleges határt, valamint az eltérést előre mutatja; a kért időt nem igazítja el csendben.
3. **Két, külön választható copy-garancia** a korábban rögzített stream-copy határokkal összhangban:
   - **Bitazonos:** a másolt tömörített minták bájtjai és sorrendje változatlan. Ha ez nem tartható, a művelet elutasítandó.
   - **Kompatibilis:** a tömörített minták dekódolt tartalma változatlan; a konténerhez szükséges időbélyegek, indexek és engedélyezett metaadatok újraírhatók.
   A garanciák között nincs automatikus átváltás. A konténer bájtjai Smart Cutnál egyik módban sem ígérhetők azonosnak.
4. A belső, biztonságosan másolható GOP-minták payloadja nem változhat. A széleken használt encodernek kompatibilis codec configot és zárt, dekódolható csatlakozási GOP-ot kell előállítania.
5. Nem támogatott profile/bitmélység/kroma, nyitott GOP, B-frame reorder, interlace, HDR, audio delay/priming vagy célmuxer esetén a terv pontos okkal leáll. Hagyományos teljes render-export csak külön felhasználói választással indulhat.
6. Az eredeti forrás nem módosul. Ideiglenes kimenet csak ellenőrzött finalizálás után nevezhető át véglegesre.

## Megközelítések

| Megközelítés | Előny | Korlát |
|---|---|---|
| Csak GOP/keyframe-határra illesztett copy | Egyszerű, gyors, payload bitazonos lehet | Nem ad képkocka-pontos vágást |
| Teljes klip újrakódolása | Egyszerű időzítés | Lassú, minőségromlást okozhat, nem GOPchop-elv |
| **GOP-tudatos hibrid (kiválasztott)** | **Képkocka-pontos határok, belső GOP-ok változatlan másolása** | Kodek- és konténerfüggő biztonsági vizsgálat szükséges |

## Terv

### 1. fázis – Képesség- és formátumleltár

**Érintett területek:** `crates/isobmff`, `crates/matroska`, `crates/mpegts`, `crates/codecs`, `crates/export`.

- Térképezd fel az egyes demux/mux API-k mintáit: payload, DTS/PTS, duration, sync/random-access típus, codec config, időalap, color metadata, audio és indexelési képesség.
- Vezess be `SmartCutCapabilities` vizsgálatot, amely a forrás+codec+profil+GOP+audio+célmuxer kombinációra `supported` vagy indokolt `unsupported` eredményt ad.
- A támogatott kimeneteket az igazolt képességek határozzák meg. MPEG-TS/PS kimenet csak a külön MPEG-TS muxer megvalósítása és tesztkapuja után kapcsolható be.
- Kezdeti célkombinációként a már meglévő saját decoder/encoder és muxer által teljes egészében támogatott H.264 8-bit 4:2:0 zárt GOP-os esetet vizsgáld meg; ez csak a képességvizsgálat igazolása után kerülhet bekapcsolásra.

**Elfogadás:** a támogatott és nem támogatott kombinációk tesztben lefedettek; elutasítás előtt nem nyílik kimeneti fájl.

### 2. fázis – Tiszta vágási tervelőállítás

**Fájl:** új `crates/export/src/smart_cut.rs`; GOP-metaadat esetén módosítás a `crates/codecs`/container API-kban.

- Definiáld a szerializálható `SmartCutRequest`, `SmartCutOptions`, `SmartCutPlan`, `CutMode`, `CopyGuarantee`, `SmartCutReport` típusokat.
- A planner bemenete a source index, a CFA4 Tick alapú In/Out, a célkonténer/codec és a policy; kimenete felsorolja a kiválasztott képkockákat, a dekódolási kezdőpontot, újrakódolandó széleket, másolt belső mintákat, hangmintákat és kimeneti timestamp-leképezést.
- A DTS decode-sorrend és PTS display-sorrend külön marad. Nyitott GOP-nál a leading frame-eket (pl. CRA/RASL viselkedés) nem lehet eldobni, vagy rossz sorrendben kiírni.
- A planner tiszta, fájlírás nélkül futó komponens legyen; adjon UI-előnézetet a tényleges időhatárról, a másolt/újrakódolt szakaszról, a garanciáról és figyelmeztetésekről.

**Tesztek:** frame-rácsra eső/nem eső Tick; első/utolsó képkocka; üres/túl hosszú/negatív tartomány; nincs random-access minta; nyitott GOP; PTS≠DTS; 33 bites időbélyeg-wrap; hiányzó hang; nem támogatott profile.

### 3. fázis – Hibrid mintamásolás és határ-GOP render

**Fájl:** `crates/export/src/smart_cut.rs`, a céltároló muxer moduljai, a meglévő encoder-regiszter.

- A planner által jóváhagyott belső GOP-okat mintánként, payload-módosítás nélkül írd ki.
- A határrégiót a legkorábbi szükséges biztonságos random-access mintától dekódold, majd csak a kért képkockákat add az encodernek.
- A határencoder kimenetének codec configja, színjellemzői, bitmélysége és GOP-zárása illeszkedjen a másolt szakaszokhoz; ha nincs ilyen garancia, elutasítás szükséges.
- Audio csak igazolt minta-/codec-határon másolható. AAC priming/padding, encoder delay, PCM mintahatár és A/V eltolás legyen explicit módon kezelve.
- Muxolás után ellenőrizd a durationt, indexet, timestamp-monotonitást, codec configot és hang/videó végpontot. Temp fájlt hiba vagy cancel esetén takarítsd.

**Tesztek:** másolt minták payload hash-e; határképkockák dekódolt összehasonlítása; A/V szinkron; codec config és seek; teljes/durva megszakítás; írási hiba/temp-file cleanup. ffmpeg/ffprobe csak külső teszt-oracle vagy fixture-készítő, nem függőség.

### 4. fázis – Engine parancs és Qt6 felület

**Érintett területek:** új `crates/engine/src/smart_cut.rs`, parancsregiszter, `python/cfa4_qt`, control-protocol dokumentáció.

- Regisztráld a `smartCut.plan`, `smartCut.run`, `smartCut.cancel` műveleteket `enabled()` indoklással, szerializált paraméterekkel és progress/cancel állapottal.
- A Qt6-ban a képkocka-pontos/GOP-határ vágás és a Bitazonos/Kompatibilis garancia külön kontroll legyen; az előnézet közölje a tényleges határt és újrakódolt GOP-okat.
- UI, CLI és control-channel ugyanazt az engine parancsot hívja; UI-ban saját média-/vágási logika nem keletkezhet.
- Projektadat ne változzon, kivéve, ha a felhasználó külön kéri a kivágás klipként történő elhelyezését.

### 5. fázis – Codec-bővítés és teljes ellenőrzés

- H.264-en túli codecet csak egyesével, saját demux/decode/encode/mux kombinációja és határfixture-jei után engedélyezz: HEVC, ProRes/DNx, MPEG-2.
- Nyitott GOP, B-frame reorder, field-coded/interlaced, HDR és eltérő timebase külön megfelelési kapuval aktiválható.
- CPU út legyen referencia; hardveres encoder opcionális, gyártófüggetlen fallbackkal.
- Futtasd az érintett crate teszteket, layer/CI ellenőrzéseket és Qt6 end-to-end tesztet a funkció bekapcsolásakor.

## Elfogadási kritériumok

- Támogatott bemeneten a képkocka-pontos kimenet pontosan a kiválasztott frame-tartományt adja.
- Belső, változatlan GOP-ok tömörített payloadja hash szerint egyezik; határképkockák a CFA4 dekódolt forrásframe-ekkel egyeznek a kódolás dokumentált tűrésén belül.
- Bitazonos és Kompatibilis mód külön választható, egyik sem vált át csendben a másikra.
- Kimenet helyesen seekelhető, lejátszható, megfelelő időtartamú és A/V szinkronú; megszakítás nem hagy sérült végleges fájlt.
- Csak igazolt codec/container/profile kombináció jelölhető támogatottnak.

## Felülvizsgálati fókusz

- Nyitott GOP és leading frame-ek (CRA/RASL): nincs hiányzó vagy decode-sorrendben rossz frame.
- B-frame reorder: a DTS/PTS-leképezés nem változtatja meg a megjelenítési sorrendet.
- AAC priming/padding, audio/video óraeltérés és timestamp-wrap: nincs A/V drift.
- Sérült minta, megtelt lemez, cancel: nincs panic, forrásvesztés vagy végleges részfájl.
- Ismeretlen codec metadata: az előnézet konkrét okkal tilt, nem indít csendes teljes transzkódot.

## Függőségek

- A korábban meghatározott Stream Copy terv két külön módja: Bitazonos és Kompatibilis.
- A Smart Cut által választott konténer tényleges muxer-képessége. MPEG-TS/PS kimenet csak a dedikált muxer elkészülte után.
- Encoder képes legyen kompatibilis határ-GOP-ot előállítani, különben az adott kombináció nem támogatott.
