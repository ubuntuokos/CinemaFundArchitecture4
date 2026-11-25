# F2-0: MQ-1 mérőeszköz és a CFA4 saját alapvonala

**Dátum:** 2026-11-25 · **Jelleg:** mérőeszköz és alapvonal, nem jelölt-eredmény és nem döntés (István jóváhagyta az indítást, 2026-11-25).
**Anyag:** minden szám **szintetikus**: a fixture a `cfa4_golden::range_meter::fixture` zárt képleteiből generálódik (nincs fájl, nincs véletlen), az igazság `f64`-ben számolt. Külső kód nem szerepel (licencszabály).
**Az F2 előregisztrációt nem módosítja.**

## Mit mér
MQ-1 (≥ 16 bit lebegő köztes pontosság) látható következménye: a tartományon kívüli érték (> 1,0 vagy < 0) megmarad egy rétegsor után. A mérő három premultiplikált, lineáris RGBA réteget ad (értékek −0,2 … 4,0, a kompozit −0,16 … 2,57), kiszámolja a Normal (over) eredményt `f64`-ben, és összeveti egy `f32` rögzítéssel (`RangeFrame`; az F0V rögzítő `u16`, display-kódolt, ezt nem hordozhatja).

Az MQ-1 PASS feltétele ezen az anyagon: minden tartományon kívüli minta hibája az `f32`-kerekítési határon belül (8 ulp, `max(1, |igazság|)`-ra számolva); hiányzó vagy elromlott mérés, illetve tartományon belüli fixture sosem PASS.

## Önteszt (F0V szabály: a mérő bizonyítja, hogy elbukhat)
Az igazság `f32`-másolata PASS. A rétegenként egész számban tároló csővezetékek emulációja (8 bit; 16 bit, 1,0 = 32768; 16 bit, 1,0 = 65535) mind FAIL. Ez emuláció a mérő érzékenységének bizonyítására, nem egyetlen jelölt modellje.

## CFA4 CPU-kompozitor, alapvonal (64×32, 3 réteg, Normal over)

| Mérés | Érték (szintetikus) |
|---|---|
| minták (RGB) | 6144 |
| tartományon kívüli minták | 3856 |
| legnagyobb hiba tartományon belül | 1,25e-7 |
| legnagyobb hiba tartományon kívül | 4,74e-7 (a határa 1,94e-6) |
| átlagos hiba | 4,45e-8 |
| 0..1-be vágott tartományon kívüli minta | 0 |
| **MQ-1 ezen az anyagon** | **PASS** |

**Megfigyelés (nem hibaállítás):** a Normal módon kívüli keverési módok a display-kódolt értéken számolnak, és az eredményt szándékosan [0, 1]-re vágják (`render::blend`). Egy 3,0-s alap felett egy átlátszatlan Screen réteg legfeljebb 1,0-t ad (a fény elvész), míg egy 50%-os Normal réteg 1,75-öt. MQ-1 tehát a Normal módon át teljesül, a többi módon át nem; ez a CFA4 tervezett viselkedése, de a jelöltek összehasonlításánál ismerni kell.

## Mi nincs benne
Nem tartalmaz jelöltmérést (az EC-jelölt tárolója nincs hozzáadva a munkamenethez); sebességet, GPU-utat és gépközi zajt (referenciagép PENDING); színterek (MQ-3), alfa (MQ-4) és lineáris fényű keverés (MQ-8) mérését. A CFA4-ben nincs 8/16/32 bites projektbeállítás: a kompozit út mindig `f32`; a kvantáló viselkedés a jelölteknél (F1: 8/16 bites projektben rétegenként kvantál) a mérőn át mérendő.

## Hogyan futtatható
`cargo test -p cfa4-golden --lib range_meter` (mérő és önteszt), `cargo test -p cfa4-golden --test range_meter -- --nocapture` (az alapvonal számai).
