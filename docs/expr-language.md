# CFX v1 – a CFA4 térbeli kifejezésnyelv leírása (X0)

> **Állapot:** JÓVÁHAGYVA (István, 2026-11-22 18:57), X0 elfogadva; a [CFA4-AX-INTERPRETER-001](plans/CFA4-AX-INTERPRETER-001-0.1.md) terv szerint. **Kód még nincs**: ez a szöveg a nyelv szerződése, amelyet X1 valósít meg és tesztel.
> A CFX az OpenVDB AX ötletéből indul, de **nem AX-kompatibilis**; az AX forrását és szövegét nem használtuk.

## 1. Mire való

Egy kifejezés **egy pontra vagy egy voxelre** ad értéket, és ennek attribútumát írja. Nincs globális állapot, nincs I/O. Ugyanaz a kifejezés minden elemen önállóan fut, ezért az eredmény nem függ a szálak számától.

## 2. Felépítés

```
reads @density, @temperature;     // olvasott attribútumok (kötelező, ha használja)
writes @color;                    // írt attribútumok (kötelező, ha használja)
param float gain = 1.0;           // felhasználói paraméter, az Effect Controlsban jelenik meg

float t = fit(@temperature, 300.0, 1200.0, 0.0, 1.0);
if (@density > 0.01) {
    @color = lerp(vec3(0.1, 0.0, 0.0), vec3(1.0, 0.6, 0.1), t) * gain;
} else {
    @color = vec3(0.0, 0.0, 0.0);
}
```

- A fejléc (`reads`, `writes`, `param`) a program elején áll, a törzs utasítások sora.
- Megjegyzés: `// sor` és `/* blokk */`.
- Az utasítások `;`-vel végződnek, a blokk `{ }`.
- Az azonosító: `[A-Za-z_][A-Za-z0-9_]*`; az attribútum `@` előtaggal.

## 3. Típusok

| Típus | Tartalom | Megjegyzés |
|---|---|---|
| `float` | 32 bites lebegőpontos | NaN lehetséges (lásd 8.) |
| `int` | 32 bites előjeles egész | túlcsordulás futási hiba |
| `bool` | `true`/`false` | |
| `vec3` | három `float` | `v.x`, `v.y`, `v.z`; `vec3(a,b,c)`, `vec3(a)` |

Implicit konverzió nincs (kivéve a `float` literál egész jegyekkel, pl. `1`-et `float` helyen elfogad a fordító figyelmeztetéssel). Kifejezett: `float(i)`, `int(f)` (nullához vág), `bool` csak összehasonlításból.

## 4. Operátorok

Precedencia, erősebbtől a gyengébbig: `()` · `!` `-` (egyoperandusú) · `*` `/` `%` · `+` `-` · `<` `<=` `>` `>=` · `==` `!=` · `&&` · `||` · `?:` · `=`.

- `vec3` ± `vec3`, `vec3` * `float`, `float` * `vec3`, `vec3` / `float` komponensenként; `vec3` * `vec3` komponensenként.
- `&&` és `||` rövidzár; a `?:` csak a kiválasztott ágat értékeli.
- Egész osztás nullával és `%` nullával: **futási hiba** (nem pánik).

## 5. Utasítások

| Forma | Szabály |
|---|---|
| `T x = e;` | helyi változó, a blokkja végéig él |
| `x = e;` / `@a = e;` | értékadás; az attribútum típusa az azonos típusú deklarációé |
| `if (c) {…} else {…}` | a feltétel `bool` |
| `for i in 0..N {…}` | `N` **fordításkor ismert egész konstans**, `1 ≤ N ≤ 1024`; `i` csak olvasható `int` |

Nincs `while`, `break`, `continue`, rekurzió, felhasználói függvény, string, dinamikus tömb, nem helyi ugrás. Egy kifejezés beágyazott `for`-jainak szorzata is legfeljebb 1 048 576 (fordítási hiba, ha több).

## 6. Kontextusok

**Volume-kontextus** (egy aktív voxelre; a háttérértékű blokkokat nem járja be):

| Név | Típus | Jelentés |
|---|---|---|
| `P` | `vec3` | a voxel középpontjának világpozíciója |
| `ijk` | `vec3` (egész értékekkel) | index; `int(ijk.x)` |
| `voxelsize` | `float` | élhossz |
| `@név` | a grid típusa | a grid azonos nevű attribútuma |

**Pont-kontextus:** `P`, `id` (`int`), `@név` oszlopok. Olvasás és írás **csak az aktuális elemre**. Egy kifejezés egy futásban csak egyféle kontextusban fut; a fordító a beépített nevek használatából és a fejlécből ellenőrzi.

Szabályok:
- Deklarálatlan attribútum olvasása vagy írása fordítási hiba.
- Ha egy attribútum szerepel a `writes`-ban, de nincs minden ágon értéke, a kimeneti alapérték (grid háttér / oszlop alapérték) marad; ez nem hiba.
- Egy attribútum nem szerepelhet a `reads` és a `writes` listában is, **hacsak** nem `reads @a; writes @a;` (a kifejezés az előző értékét olvassa és felülírja; az olvasott érték mindig a futás előtti érték).

## 7. Beépített függvények

Zárt lista; minden függvény teljes (minden bemenetre értéket ad).

| Csoport | Függvények |
|---|---|
| Szám | `abs min max clamp floor ceil round sqrt pow exp log sin cos tan atan2` |
| Átfedés | `lerp(a,b,t)`, `smoothstep(e0,e1,x)`, `step(e,x)`, `fit(x,a,b,c,d)` |
| Vektor | `length dot cross normalize` |
| Zaj/hash | `noise(vec3) → float` (determinisztikus gradiens-zaj, érték −1…1), `hash(int, …) → int` (legfeljebb 4 argumentum), `rand(int seed, int id) → float` (0…1, determinisztikus) |

A függvények `float`-on és (ahol értelmes) `vec3`-on komponensenként működnek; a pontos aláírásokat X1 teszti egy táblázat alapján.

## 8. Definiálatlan esetek (a „nincs pánik” szerződés)

| Bemenet | Eredmény |
|---|---|
| `sqrt(x)`, x < 0 | `NaN` |
| `log(x)`, x ≤ 0 | `NaN` |
| `pow(0, negatív)` | `+inf` |
| `normalize(0-vektor)` | `vec3(0,0,0)` |
| `atan2(0,0)` | `0` |
| `fit` ha `a == b` | `c` |
| `int` túlcsordulás, osztás/modulo nullával | futási hiba (a hely megjelölésével) |
| `NaN` az írt attribútumban | a kimeneten jelölt (voxelenként számolva, a figyelmeztetés a futás végén); **nem** terjed csendben tovább az eredményen kívülre |

A `NaN` és az `inf` bit-szinten determinisztikus (a kanonikus NaN-t írjuk ki).

## 9. Hibakatalógus

Minden hiba: `{kód, sor, oszlop, hossz, üzenet}`; a kód stabil, az üzenet magyar és angol.

| Kód | Mikor |
|---|---|
| `E-LEX-*` | ismeretlen karakter, lezáratlan megjegyzés, túl hosszú azonosító |
| `E-PARSE-*` | szintaktikai hiba (a legközelebbi várt jelet megnevezi) |
| `E-TYPE-*` | típus-eltérés, ismeretlen függvény/név, rossz argumentumszám |
| `E-ATTR-*` | deklarálatlan attribútum, írás nem `writes`-ba, ismeretlen kontextus-név |
| `E-LOOP-*` | nem konstans vagy tartományon kívüli ciklushatár, iterációszorzat > 1 048 576 |
| `E-LIMIT-*` | forrás-, utasítás- vagy kifejezésmélység-korlát túllépve |
| `R-FUEL` | a voxelenkénti utasításkorlát kimerült |
| `R-INT` | `int` túlcsordulás vagy nullával osztás |
| `R-CANCEL` | felhasználói megszakítás |

A fordító **az első** hibánál megáll X1-ben; a több hibát gyűjtő mód későbbi bővítés.

## 10. Korlátok (alapérték, a projekt-beállításban csökkenthető)

| Korlát | Érték |
|---|---|
| forrás mérete | 64 KiB |
| kifejezésfa mélysége | 64 |
| helyi változók száma | 256 |
| `for` iteráció (egy ciklus) | 1024; beágyazva a szorzat ≤ 1 048 576 |
| fuel egy voxelre | 10 000 bájtkód-utasítás (a kemény felső határ: 1 000 000) |

## 11. Példák (X1 teszt-korpusz alapja)

**11.1 Küszöbölés**
```
reads @density; writes @mask;
@mask = @density > 0.5 ? 1.0 : 0.0;
```

**11.2 Sebességmező zajból**
```
writes @vel;
param float scale = 0.05;
param float amp = 2.0;
vec3 q = P * scale;
@vel = vec3(noise(q), noise(q + vec3(31.4, 0.0, 0.0)), noise(q + vec3(0.0, 17.2, 0.0))) * amp;
```

**11.3 Korlátos ciklus**
```
reads @density; writes @out;
float acc = 0.0;
for i in 0..8 {
    acc = acc + @density * float(i) * 0.125;
}
@out = acc;
```

**11.4 Pontok színezése**
```
writes @Cd;
float h = fit(P.y, 0.0, 10.0, 0.0, 1.0);
@Cd = lerp(vec3(0.0, 0.2, 1.0), vec3(1.0, 0.3, 0.0), clamp(h, 0.0, 1.0));
```

**11.5 Elutasítandó (a hibakatalógus tesztjei)**
```
@a = 1.0;                   // E-ATTR: @a nincs a writes-ban
for i in 0..n { }           // E-LOOP: nem konstans határ
float x = 1;                // figyelmeztetés: egész jegyű literál float helyén
int j = 2147483647 + 1;     // R-INT futáskor (nem fordításkor, ha nem konstans-hajtogatható)
open("/etc/passwd");        // E-TYPE: nincs ilyen függvény
```

## 12. Nem cél (v1)

Szomszéd-hozzáférés (v2, Q-X1), felhasználói függvények, stringek, tömbök, GPU-futtatás, AX-szkript importja (Q-X4).
