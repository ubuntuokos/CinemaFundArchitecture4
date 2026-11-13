//! Reading Cinecred projects (C7): a project folder holds `Styling.toml` and one or more credits
//! spreadsheets. The spreadsheet columns, inline tags and page behaviours mean what they mean in
//! CFA4's own sheets (see [`crate::sheet`]); this module adds Cinecred's column and tag names in
//! its languages, and turns `Styling.toml` into a CFA4 [`Styling`] as closely as CFA4's styles
//! allow, listing what could not be carried over.
//!
//! The file format was learnt from Cinecred's documentation and its published source (GPL-3.0-or
//! -later, like CFA4; registered in `docs/licensing.md` §5). No Cinecred code is copied: this is
//! an independent reader of the format.

use crate::sheet::Column;
use crate::style::{Align, BodyLayout, ContentStyle, LetterStyle, Orientation, PageStyle, Styling};
use crate::{Behaviour, CreditsError};

/// The styling file of a Cinecred project.
pub const STYLING_FILE: &str = "Styling.toml";
/// Largest styling file read.
pub const MAX_STYLING_BYTES: usize = 4 << 20;
/// Most styles of one kind read.
const MAX_STYLES: usize = 1_000;

/// Column names Cinecred uses (its languages), for [`Column::from_header`].
const COLUMNS: &[(Column, &[&str])] = &[
    (Column::Head, &["Cabeçalho", "Encabezado", "Entête", "Hlavička", "Hode", "Kopf", "Голова", "Заголовок", "தலை", "页眉"]),
    (Column::Body, &["Corpo", "Corps", "Cuerpo", "Hovedinnhold", "Körper", "Tělo", "Туловище", "Тіло", "உடல்", "正文"]),
    (Column::Tail, &["Base", "Cauda", "Fuß", "Hale", "Patka", "Pied", "Хвост", "Хвіст", "வால்", "页脚"]),
    (
        Column::Gap,
        &[
            "Espace vertical",
            "Espaçamento Vertical",
            "Senkrechte Lücke",
            "Separación Vertical",
            "Vertikal mellomrom",
            "Vertikální mezera",
            "Вертикальний Інтервал",
            "Вертикальный отступ",
            "செங்குத்து இடைவெளி",
            "纵向间距",
            "Vert. Gap",
            "Senkr. Lücke",
        ],
    ),
    (
        Column::Style,
        &[
            "Estilo del Contenido",
            "Estilo do Conteúdo",
            "Inhaltsstil",
            "Innholdsstil",
            "Styl obsahu",
            "Style de contenu",
            "Стиль Змісту",
            "Стиль контента",
            "உள்ளடக்க நடை",
            "内容样式",
        ],
    ),
    (
        Column::Spine,
        &[
            "Posición del Eje",
            "Position de la ligne guide",
            "Posição da Espinha",
            "Pozice páteře",
            "Ryggposisjon",
            "Rückgratposition",
            "Позиция оси",
            "Позиція Хребта",
            "முதுகெலும்பு நிலை",
            "轴线位置",
            "Column Pos.",
            "Spaltenposition",
        ],
    ),
    (
        Column::Page,
        &[
            "Estilo da Página",
            "Estilo de Página",
            "Seitenstil",
            "Sidestil",
            "Styl stránky",
            "Style de page",
            "Стиль Сторінки",
            "Стиль страницы",
            "பக்க நடை",
            "页面样式",
        ],
    ),
    (
        Column::Runtime,
        &[
            "Durée d’exécution de la page",
            "Seitenlaufzeit",
            "Sidespilletid",
            "Tempo de Execução da Página",
            "Tiempo de Ejecución de la Página",
            "Trvání stránky",
            "Время выполнения страницы",
            "Час Виконання Сторінки",
            "பக்க இயக்க நேரம்",
            "页面时长",
        ],
    ),
    (
        Column::PageGap,
        &[
            "Espacio de la Página",
            "Espaçamento entre Páginas",
            "Mezera stránky",
            "Seitenlücke",
            "Sidemellomrom",
            "Écart de page",
            "Інтервал Сторінки",
            "Промежуток между страницами",
            "பக்க இடைவெளி",
            "页面间距",
        ],
    ),
];

/// Tag words in Cinecred's languages (lower case).
const STYLE_WORDS: &[&str] = &["estilo", "stil", "styl", "стиль", "சூல் தண்டு", "样式"];
const PIC_WORDS: &[&str] = &["bild", "bilde", "image", "imagen", "img", "obrázek", "зображення", "изображение", "படம்", "图像"];
const VIDEO_WORDS: &[&str] = &["vidéo", "vídeo", "видео", "відео", "ஒளிதோற்றம்", "视频"];
const BLANK_WORDS: &[&str] = &["blank", "leer", "prázdné", "tom", "vacuo", "vazio", "vide", "пустий", "пусто", "வெற்று", "空白"];
const PARALLEL_WORDS: &[&str] = &["paralela", "paralelo", "parallell", "parallèle", "souběžně", "паралельно", "параллельно", "இணையான", "平行"];

fn norm_header(s: &str) -> String {
    s.trim().trim_start_matches('@').to_lowercase().chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect()
}

/// A column by one of Cinecred's names.
pub fn column(header: &str) -> Option<Column> {
    let n = norm_header(header);
    COLUMNS.iter().find(|(_, names)| names.iter().any(|x| norm_header(x) == n)).map(|(c, _)| *c)
}

pub fn is_style_word(w: &str) -> bool {
    STYLE_WORDS.contains(&w)
}

/// A style tag whose keyword is several words (Tamil): the rest of the tag after it.
pub fn multiword_style(tag: &str) -> Option<&str> {
    let t = tag.trim();
    STYLE_WORDS.iter().filter(|w| w.contains(' ')).find_map(|w| {
        let rest = t.strip_prefix(w)?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(rest.trim())
    })
}
pub fn is_pic_word(w: &str) -> bool {
    PIC_WORDS.contains(&w)
}
pub fn is_video_word(w: &str) -> bool {
    VIDEO_WORDS.contains(&w)
}
/// `{{Blank}}` in any of Cinecred's languages (and `{{Üres}}`).
pub fn is_blank_word(w: &str) -> bool {
    let w = w.trim().to_lowercase();
    BLANK_WORDS.contains(&w.as_str()) || w == "üres"
}
pub fn is_parallel_word(w: &str) -> bool {
    PARALLEL_WORDS.contains(&w)
}

/// A Cinecred styling as CFA4 styles, and what was left out.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub styling: Styling,
    /// What the project's pixels are worth on a 1080-line frame (1080 / its height): the sheet's
    /// own pixel values are scaled by this too ([`crate::doc::CreditsDoc::pixel_scale`]).
    pub scale: f64,
    /// Plain-English notes on what could not be carried over.
    pub notes: Vec<String>,
}

type Table = toml::Table;

fn get_str<'a>(t: &'a Table, k: &str) -> Option<&'a str> {
    t.get(k).and_then(toml::Value::as_str)
}
fn get_f(t: &Table, k: &str) -> Option<f64> {
    t.get(k).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).filter(|v| v.is_finite())
}
fn get_i(t: &Table, k: &str) -> Option<i64> {
    t.get(k).and_then(|v| v.as_integer().or_else(|| v.as_float().filter(|f| f.is_finite()).map(|f| f.round() as i64)))
}
fn get_b(t: &Table, k: &str) -> Option<bool> {
    t.get(k).and_then(toml::Value::as_bool)
}
fn tables<'a>(root: &'a Table, k: &str) -> Vec<&'a Table> {
    root.get(k).and_then(toml::Value::as_array).map(|a| a.iter().filter_map(toml::Value::as_table).take(MAX_STYLES).collect()).unwrap_or_default()
}

/// sRGB `#rrggbb` from Cinecred's colour: `[X, Y, Z, alpha]` in CIE XYZ (D50), linear.
fn xyz_d50_to_hex(v: &[toml::Value]) -> Option<String> {
    let n: Vec<f64> = v.iter().take(3).filter_map(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64))).collect();
    let [x, y, z] = <[f64; 3]>::try_from(n).ok()?;
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return None;
    }
    // Bradford-adapted XYZ (D50) to linear sRGB (D65)
    let r = 3.133_856_1 * x - 1.616_866_7 * y - 0.490_614_6 * z;
    let g = -0.978_768_4 * x + 1.916_141_5 * y + 0.033_454 * z;
    let b = 0.071_945_3 * x - 0.228_991_4 * y + 1.405_242_7 * z;
    let enc = |l: f64| {
        let l = l.clamp(0.0, 1.0);
        let s = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Some(format!("#{:02x}{:02x}{:02x}", enc(r), enc(g), enc(b)))
}

/// Font weight and slant words that end a full font name ("Inter Semi Bold Italic").
const STYLE_SUFFIXES: &[&str] = &[
    "thin",
    "hairline",
    "extralight",
    "ultralight",
    "light",
    "regular",
    "normal",
    "book",
    "medium",
    "semibold",
    "demibold",
    "bold",
    "extrabold",
    "ultrabold",
    "black",
    "heavy",
    "italic",
    "oblique",
    "semi",
    "demi",
    "extra",
    "ultra",
];

/// A full font name split into family and style.
fn split_font(full: &str) -> (String, String) {
    let words: Vec<&str> = full.split_whitespace().collect();
    let mut k = words.len();
    while k > 1 && words.get(k - 1).is_some_and(|w| STYLE_SUFFIXES.contains(&w.to_lowercase().as_str())) {
        k -= 1;
    }
    let family = words.get(..k).map(|w| w.join(" ")).unwrap_or_default();
    let style: String = words.get(k..).map(|w| w.concat()).unwrap_or_default();
    (if family.is_empty() { full.to_string() } else { family }, if style.is_empty() { "Regular".into() } else { style })
}

/// A Cinecred letter style as a CFA4 letter style (sizes in pixels of a 1080-high frame).
fn letter(t: &Table, scale: f64, notes: &mut Vec<String>) -> LetterStyle {
    let mut ls = LetterStyle::default();
    if let Some(f) = get_str(t, "font") {
        let (family, style) = split_font(f);
        ls.font = family;
        ls.font_style = style;
    }
    // Cinecred gives the line's whole height; the leading ratios take the space above and below
    let height = get_f(t, "heightPx").unwrap_or(32.0).clamp(1.0, 10_000.0) * scale;
    let lead = (get_f(t, "leadingTopRh").unwrap_or(0.0) + get_f(t, "leadingBottomRh").unwrap_or(0.0)).clamp(0.0, 0.9);
    let font_height = height * (1.0 - lead);
    // a font's ascent and descent together are about 1.2 em
    ls.size = (font_height / 1.2).clamp(1.0, 2_000.0);
    ls.leading = (height - font_height).max(0.0);
    ls.tracking = get_f(t, "trackingEm").unwrap_or(0.0) * 1000.0;
    ls.all_caps = get_b(t, "uppercase").unwrap_or(false);
    // the colour of the first coloured layer
    let colour = t.get("layers").and_then(toml::Value::as_array).and_then(|layers| {
        layers.iter().filter_map(toml::Value::as_table).find_map(|l| match get_str(l, "coloring").map(str::to_uppercase).as_deref() {
            Some("OFF") => None,
            Some("GRADIENT") => {
                l.get("gradientStops").and_then(toml::Value::as_array).and_then(|s| s.first()).and_then(toml::Value::as_array).and_then(|c| xyz_d50_to_hex(c))
            }
            _ => l.get("plainColor").and_then(toml::Value::as_array).and_then(|c| xyz_d50_to_hex(c)),
        })
    });
    if let Some(c) = colour {
        ls.color = c;
    }
    if t.get("layers").and_then(toml::Value::as_array).is_some_and(|l| l.len() > 1) && notes.len() < 100 {
        notes.push(format!("letter style `{}`: only the first coloured layer is used (no outlines, shadows or stripes)", get_str(t, "name").unwrap_or("?")));
    }
    ls
}

/// Read a Cinecred `Styling.toml`.
pub fn read_styling(text: &str) -> Result<Imported, CreditsError> {
    if text.len() > MAX_STYLING_BYTES {
        return Err(CreditsError::TooLarge(format!("the Cinecred styling is larger than {} MiB", MAX_STYLING_BYTES >> 20)));
    }
    let root: Table = text.parse().map_err(|e: toml::de::Error| CreditsError::Styling(format!("Styling.toml: {}", e.message())))?;
    let mut notes = Vec::new();
    let global = root.get("global").and_then(toml::Value::as_table).cloned().unwrap_or_default();
    // Cinecred sizes are pixels of its own resolution; CFA4's of a 1080-high frame
    let height = get_str(&global, "resolution")
        .and_then(|r| r.split_once('x'))
        .and_then(|(_, h)| h.trim().parse::<f64>().ok())
        .filter(|h| h.is_finite() && *h >= 1.0 && *h <= 100_000.0)
        .unwrap_or(1080.0);
    let scale = 1080.0 / height;
    let mut st = Styling { unit_px: get_f(&global, "unitVGapPx").unwrap_or(32.0).clamp(0.0, 10_000.0) * scale, ..Styling::default() };
    if get_i(&global, "runtimeFrames").is_some() {
        notes.push("the whole-credits runtime is not carried over; use Fit Credits to Music (or a length) instead".into());
    }

    // letter styles by name
    let letters: Vec<(String, LetterStyle)> =
        tables(&root, "letterStyle").into_iter().map(|t| (get_str(t, "name").unwrap_or("").to_string(), letter(t, scale, &mut notes))).collect();
    let letter_named = |name: Option<&str>| name.and_then(|n| letters.iter().find(|(k, _)| k == n).map(|(_, l)| l.clone()));
    // every letter style can also be used inline: {{Style Name}}
    st.inline_styles = letters
        .iter()
        .filter(|(n, _)| !n.is_empty())
        .map(|(n, l)| crate::markup::InlineStyle {
            name: n.clone(),
            font: Some(l.font.clone()),
            font_style: Some(l.font_style.clone()),
            size: Some(l.size),
            color: Some(l.color.clone()),
            tracking: Some(l.tracking),
            all_caps: Some(l.all_caps),
            ..Default::default()
        })
        .collect();

    for t in tables(&root, "pageStyle") {
        let name = get_str(t, "name").unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        let behaviour = match get_str(t, "behavior").map(str::to_uppercase).as_deref() {
            Some("CARD") => Behaviour::Card,
            _ => Behaviour::Scroll,
        };
        let frames = |k: &str, d: u32| get_i(t, k).map(|v| v.clamp(0, 1_000_000) as u32).unwrap_or(d);
        st.page_styles.push(PageStyle {
            name: name.clone(),
            behaviour,
            card_frames: frames("cardRuntimeFrames", 120).max(1),
            fade_in_frames: frames("cardFadeInFrames", 12),
            fade_out_frames: frames("cardFadeOutFrames", 12),
            scroll_px_per_frame: get_f(t, "scrollPxPerFrame").unwrap_or(3.0).clamp(0.01, 10_000.0) * scale,
            gap_after_frames: get_i(t, "subsequentGapFrames").unwrap_or(0).clamp(-1_000_000, 1_000_000) as i32,
            ..PageStyle::default()
        });
        if get_str(t, "cardFadeInTransitionStyleName").is_some_and(|s| !s.is_empty())
            || get_str(t, "cardFadeOutTransitionStyleName").is_some_and(|s| !s.is_empty())
        {
            notes.push(format!("page style `{name}`: fade curves become linear fades"));
        }
    }

    for t in tables(&root, "contentStyle") {
        let name = get_str(t, "name").unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        let px = |k: &str, d: f64| get_f(t, k).unwrap_or(d).clamp(0.0, 100_000.0) * scale;
        let body = letter_named(get_str(t, "bodyLetterStyleName")).unwrap_or_default();
        let head = letter_named(get_str(t, "headLetterStyleName")).unwrap_or_else(|| body.clone());
        let tail = letter_named(get_str(t, "tailLetterStyleName")).unwrap_or_else(|| body.clone());
        let layout = match get_str(t, "bodyLayout").map(str::to_uppercase).as_deref() {
            Some("FLOW") => BodyLayout::Flow,
            Some("PARAGRAPHS") => BodyLayout::Paragraphs,
            _ => BodyLayout::Grid,
        };
        let justify = |k: &str| match get_str(t, k).map(str::to_uppercase).as_deref() {
            Some("LEFT") => Some(Align::Left),
            Some("RIGHT") => Some(Align::Right),
            Some("CENTER") => Some(Align::Center),
            _ => None,
        };
        let first_col =
            t.get("gridCellHJustifyPerCol").and_then(toml::Value::as_array).and_then(|a| a.first()).and_then(toml::Value::as_str).map(str::to_uppercase);
        let align = match first_col.as_deref() {
            Some("LEFT") => Align::Left,
            Some("RIGHT") => Align::Right,
            _ => Align::Center,
        };
        let horizontal = get_str(t, "blockOrientation").is_some_and(|o| o.eq_ignore_ascii_case("HORIZONTAL"));
        let harmonize = get_str(t, "headHarmonizeWidth").is_some_and(|h| h.eq_ignore_ascii_case("ACROSS_BLOCKS"));
        st.content_styles.push(ContentStyle {
            name: name.clone(),
            orientation: if horizontal { Orientation::Horizontal } else { Orientation::Vertical },
            layout,
            columns: get_i(t, "gridCols").unwrap_or(1).clamp(1, 16) as u32,
            align,
            body,
            head,
            tail,
            head_gap: px("headGapPx", 12.0),
            tail_gap: px("tailGapPx", 12.0),
            column_gap: px("gridColGapPx", 64.0),
            paragraph_gap: px("paragraphsParaGapPx", 24.0),
            wrap_width: match layout {
                BodyLayout::Flow => px("flowRowWidthPx", 1200.0),
                _ => px("paragraphsLineWidthPx", 1200.0),
            },
            flow_separator: get_str(t, "flowSeparator").unwrap_or("  •  ").to_string(),
            margin: px("vMarginTopPx", 0.0).max(px("vMarginBottomPx", 0.0)),
            head_align: justify("headHJustify").unwrap_or(Align::Right),
            harmonize_heads: harmonize,
            leader: get_str(t, "headLeader").unwrap_or("").to_string(),
            leader_gap: px("headLeaderMarginLeftPx", 16.0),
            ..ContentStyle::default()
        });
        if get_b(t, "hasHead") == Some(false) && notes.len() < 100 {
            notes.push(format!("content style `{name}` has no head in Cinecred; heads written in the sheet are still shown"));
        }
        if get_str(t, "sort").is_some_and(|s| !s.eq_ignore_ascii_case("OFF")) && notes.len() < 100 {
            notes.push(format!("content style `{name}`: sorting the names is not carried over"));
        }
    }
    for (kind, what) in
        [("transitionStyle", "transition curves"), ("pictureStyle", "picture styles (pictures keep their own size)"), ("tapeStyle", "video (tape) styles")]
    {
        if !tables(&root, kind).is_empty() {
            notes.push(format!("{what} are not carried over"));
        }
    }
    if let Some(first) = st.page_styles.first() {
        st.default_page_style = first.name.clone();
    }
    if let Some(first) = st.content_styles.first() {
        st.default_content_style = first.name.clone();
    }
    Ok(Imported { styling: st, scale, notes })
}
