# A tulajdonos referenciagépe

**Tulajdonos:** ubuntuokos · **Rögzítve:** 2026-11-02 (a tulajdonos adatai alapján)

Ez a CFA4 fejlesztő- és referenciagépe. Minden ember és MI-munkamenet innen veszi a gép adatait,
ne kérdezze meg újra a tulajdonostól. Ha a gép változik, ezt a fájlt kell frissíteni.

| | |
|---|---|
| Gyártó, típus | Dell Inc. Precision Tower 7910 |
| Processzor | 2 × Intel Xeon E5-2696 v4 @ 2,20 GHz (2 × 22 mag, Hyper-Threadinggel 88 szál; Broadwell-EP, 2 NUMA-csomópont) |
| Memória | 512 GiB RAM (503,8 GiB használható) |
| GPU | NVIDIA RTX A1000 (csak monitorhoz) + NVIDIA RTX A4000 (csak számításra) |
| Hálózat | nincs 10 GbE-s vagy PTP hardveres időbélyeget tudó kártya (a tulajdonos közlése, 2026-11-11; ST 2110-mérésekhez számít) |
| Operációs rendszer | Ubuntu Studio 26.04 (AMD64) |
| Asztali környezet | KDE Plasma 6.6.6, KDE Frameworks 6.24.0 |
| Grafikai platform | Wayland |
| Qt | 6.10.2 |
| Rendszermag | 7.0.0-43-generic (64 bit) |

## Mérési következmények

- **Minimumgép-mérés:** a gép sokszorosa a minimális kliensgépnek, ezért minimumra vonatkozó
  mérés csak korlátozott futtatással érvényes (cgroup-korlát RAM-ra swap nélkül, kiválasztott
  magok egy NUMA-csomóponton). A meglévő minta: `CFA4-AI_modules` `scripts/dev/run-minspec.sh`
  (ADR-0006: „a fejlesztőgép 512 GB-os RAM-ja elfedné a memórianyomásból eredő hibákat”).
- **Magonkénti teljesítmény:** a Broadwell-magok egyenként gyengébbek egy mai asztali vagy laptop
  CPU magjainál, így egy kevés magra korlátozott mérés a biztonságos (lassabb) irányba téved.
- **Hyper-Threading:** a 88 szálas gépen a `taskset -c 0-7` nyolc külön fizikai magot ad, nem
  4 magot / 8 szálat. A HT-párokat az `lscpu -e` mutatja.
- **CPU-only mérés:** mindkét NVIDIA kártyát el kell rejteni (és `--hw off`), különben a GPU-s út fut.
- **GPU-mérés:** a számítási kártya az RTX A4000; az RTX A1000 a kijelzőt hajtja. A korábbi
  RTX A1000-es auditok (R0, F7, F9) bizonyítékai érvényesek maradnak.
- Minden mérési jegyzőkönyv rögzítse az `lscpu` kimenetét, a rögzített magokat és a terhelést
  ([`VISION.md`](../../VISION.md) 2.5, [`docs/performance.md`](../performance.md)).
