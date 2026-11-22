//! UI translations. The English text is the key, so a missing translation
//! falls back to English instead of breaking. To add a language: add a
//! `Lang` variant and one column to `TABLE`.
//!
//! Templates use named placeholders (`{n}`, `{name}`) filled by [`tf`].

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ro,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ro];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ro => "ro",
        }
    }

    pub fn from_code(c: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == c)
    }

    /// Name shown in the picker, in the language itself.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ro => "Română",
        }
    }

    /// From `LC_ALL` / `LC_MESSAGES` / `LANG`, e.g. "ro_RO.UTF-8".
    pub fn system() -> Lang {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .and_then(|v| Lang::from_code(&v[..v.len().min(2)].to_lowercase()))
            .unwrap_or(Lang::En)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_current(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Ro,
        _ => Lang::En,
    }
}

/// Apply the language from settings (or the system one).
pub fn apply_setting(choice: Option<Lang>) {
    set_current(choice.unwrap_or_else(Lang::system));
}

/// Translate a UI string (English text is the key).
pub fn t(en: &'static str) -> &'static str {
    match current() {
        Lang::En => en,
        Lang::Ro => TABLE
            .iter()
            .find(|(k, _)| *k == en)
            .map(|(_, ro)| *ro)
            .unwrap_or(en),
    }
}

/// Translate a template and fill `{name}` placeholders.
pub fn tf(en: &'static str, args: &[(&str, &str)]) -> String {
    let mut s = t(en).to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// (English, Română)
static TABLE: &[(&str, &str)] = &[
    // shell / toolbar / status
    ("Search", "Caută"),
    ("New", "Nou"),
    ("Lock", "Blochează"),
    ("Settings", "Setări"),
    ("Saved", "Salvat"),
    ("Key in memory", "Cheia în memorie"),
    ("The vault key could not be locked in RAM (memory lock limit too low, or a v2 vault), so it can reach swap. Use encrypted swap, or raise RLIMIT_MEMLOCK.", "Cheia seifului nu a putut fi blocată în RAM (limita de memorie blocată e prea mică sau seiful e v2), deci poate ajunge în swap. Folosește swap criptat sau mărește RLIMIT_MEMLOCK."),
    ("The vault key is not locked in RAM on Windows yet, so it can reach the page file. Use BitLocker or another full-disk encryption.", "Pe Windows cheia seifului nu este încă blocată în RAM, deci poate ajunge în fișierul de paginare. Folosește BitLocker sau altă criptare completă a discului."),
    ("Changes that could not be saved were kept in {name} (same master password).", "Modificările care nu au putut fi salvate au fost păstrate în {name} (aceeași parolă principală)."),
    ("Unsaved changes were lost: {e}", "Modificările nesalvate s-au pierdut: {e}"),
    ("Copy failed: the clipboard is not available", "Copierea a eșuat: clipboardul nu este disponibil"),
    ("Not opened: misleading address (real host {host})", "Nu se deschide: adresă înșelătoare (gazda reală {host})"),
    ("Opens {host}", "Deschide {host}"),
    ("This file was a Password Safe v3 vault and is now an old v2 file. It may have been replaced with an outdated copy (check the .ibak backup or your sync history). Editing is off.", "Acest fișier era un seif Password Safe v3 și acum este un fișier v2 vechi. Poate a fost înlocuit cu o copie depășită (verificați copia .ibak sau istoricul sincronizării). Editarea este dezactivată."),
    ("Edit as v2 anyway", "Editează totuși ca v2"),
    ("Other users on this computer can read the vault file.", "Alți utilizatori ai acestui calculator pot citi fișierul seifului."),
    ("Make private", "Fă-l privat"),
    ("Dismiss", "Închide"),
    ("Could not change permissions: {e}", "Nu s-au putut schimba permisiunile: {e}"),
    ("Read‑only", "Doar citire"),
    ("Unsaved change", "Modificare nesalvată"),
    ("Password Safe v2 (legacy)", "Password Safe v2 (vechi)"),
    ("{n} entries", "{n} intrări"),
    ("Clipboard clears in {s}s", "Clipboardul se golește în {s}s"),
    (
        "Ctrl+N new · Ctrl+E edit · Ctrl+C password · Ctrl+B user · Ctrl+L lock",
        "Ctrl+N nou · Ctrl+E editare · Ctrl+C parolă · Ctrl+B utilizator · Ctrl+L blocare",
    ),
    (
        "⌘N new · ⌘E edit · ⌘C password · ⌘B user · ⌘L lock",
        "⌘N nou · ⌘E editare · ⌘C parolă · ⌘B utilizator · ⌘L blocare",
    ),
    // sidebar / list
    ("All entries", "Toate intrările"),
    ("Expiring", "Expiră curând"),
    ("Favorites", "Favorite"),
    ("Reused", "Refolosite"),
    ("Same password as 1 other entry.", "Aceeași parolă ca la încă o intrare."),
    ("Same password as {n} other entries.", "Aceeași parolă ca la alte {n} intrări."),
    ("Weak password: consider generating a new one.", "Parolă slabă: generați una nouă."),
    ("GROUPS", "GRUPURI"),
    ("No matching entries", "Nicio intrare găsită"),
    ("Select an entry", "Selectați o intrare"),
    ("expired", "expirat"),
    // detail
    ("Edit", "Editează"),
    ("Duplicate", "Duplică"),
    ("Delete", "Șterge"),
    ("No group", "Fără grup"),
    ("Username", "Utilizator"),
    ("Password", "Parolă"),
    ("Website", "Site web"),
    ("Email", "E‑mail"),
    ("NOTES", "NOTE"),
    ("PASSWORD HISTORY · {n}", "ISTORIC PAROLE · {n}"),
    ("set {date}", "setată {date}"),
    ("Created {date}", "Creat {date}"),
    ("Modified {date}", "Modificat {date}"),
    ("Password changed {date}", "Parolă schimbată {date}"),
    ("Password expires in {n} days ({date}).", "Parola expiră în {n} zile ({date})."),
    ("Password expires tomorrow ({date}).", "Parola expiră mâine ({date})."),
    ("Password expired today ({date}).", "Parola a expirat azi ({date})."),
    ("Password expired yesterday ({date}).", "Parola a expirat ieri ({date})."),
    ("Password expired {n} days ago ({date}).", "Parola a expirat acum {n} zile ({date})."),
    ("{label} copied", "{label} copiat"),
    ("Old password", "Parola veche"),
    // banners
    (
        "Read-only: the vault is open in another app ({who}).",
        "Doar citire: seiful este deschis în altă aplicație ({who}).",
    ),
    (
        "Read-only: the vault is open in another app ({who}) — that process is no longer running.",
        "Doar citire: seiful este deschis în altă aplicație ({who}) — procesul nu mai rulează.",
    ),
    ("Take over lock", "Preia blocarea"),
    (
        "The vault file changed on disk (another device or app). Your last change is NOT saved yet.",
        "Fișierul seifului s-a modificat pe disc (alt dispozitiv sau altă aplicație). Ultima modificare NU este încă salvată.",
    ),
    ("Overwrite file", "Suprascrie fișierul"),
    ("Discard & lock", "Renunță și blochează"),
    ("Save failed: {e}", "Salvarea a eșuat: {e}"),
    ("Lock taken over — editing enabled", "Blocare preluată — editarea este permisă"),
    ("Could not take lock: {e}", "Blocarea nu a putut fi preluată: {e}"),
    (
        "Legacy Password Safe v2 file (Password Gorilla). Kept in v2 so older apps can still open it, but v2 has weak key derivation and no tamper detection.",
        "Fișier Password Safe v2 vechi (Password Gorilla). Rămâne în v2 ca aplicațiile vechi să-l poată deschide, dar v2 are o derivare slabă a cheii și nu detectează modificările neautorizate.",
    ),
    ("Upgrade to v3…", "Actualizează la v3…"),
    // dialogs
    ("Delete “{title}”?", "Ștergeți „{title}”?"),
    (
        "The entry is removed from the vault file. The previous version of the file is kept next to it as a .ibak backup.",
        "Intrarea este eliminată din fișierul seifului. Versiunea anterioară a fișierului se păstrează alături, ca copie de rezervă .ibak.",
    ),
    ("Cancel", "Anulează"),
    ("Enter delete · Esc cancel", "Enter șterge · Esc anulează"),
    ("Upgrade to Password Safe v3", "Actualizare la Password Safe v3"),
    (
        "Keywhere writes a new file, {target}, with modern encryption (262,144 key‑stretching rounds and tamper detection) and continues with it. The v2 file stays exactly as it is. Password Gorilla 1.5+ and Password Safe 3 open v3.",
        "Keywhere scrie un fișier nou, {target}, cu criptare modernă (262.144 runde de întărire a cheii și detectarea modificărilor) și continuă cu el. Fișierul v2 rămâne neschimbat. Password Gorilla 1.5+ și Password Safe 3 deschid v3.",
    ),
    ("Upgrade", "Actualizează"),
    ("Master password", "Parola principală"),
    ("Wrong master password.", "Parolă principală greșită."),
    ("{name} already exists; move it away first.", "{name} există deja; mutați-l mai întâi."),
    ("Could not write the v3 file: {e}", "Fișierul v3 nu a putut fi scris: {e}"),
    (
        "Upgraded — now using {name} (the v2 file is unchanged)",
        "Actualizat — se folosește acum {name} (fișierul v2 este neschimbat)",
    ),
    (
        "Upgraded, but the new file's lock could not be taken; read-only until you reopen it.",
        "Actualizat, dar blocarea noului fișier nu a putut fi preluată; doar citire până la redeschidere.",
    ),
    // editor
    ("New entry", "Intrare nouă"),
    ("Edit entry", "Editare intrare"),
    ("Ctrl+Enter save · Esc cancel", "Ctrl+Enter salvează · Esc anulează"),
    ("Title", "Titlu"),
    ("Title (required)", "Titlu (obligatoriu)"),
    ("Group", "Grup"),
    ("Group — pick or type, levels separated by “ / ”", "Grup — alegeți sau scrieți, niveluri separate prin „ / ”"),
    ("New group: {name}", "Grup nou: {name}"),
    ("New entry in “{group}”", "Intrare nouă în „{group}”"),
    ("New entry (no group)", "Intrare nouă (fără grup)"),
    ("Expand", "Extinde"),
    ("New group…", "Grup nou…"),
    ("New subgroup…", "Subgrup nou…"),
    ("New group", "Grup nou"),
    ("New subgroup in “{group}”", "Subgrup nou în „{group}”"),
    ("Group name", "Numele grupului"),
    ("Create", "Creează"),
    ("Enter a name.", "Introduceți un nume."),
    ("That group already exists.", "Grupul există deja."),
    ("Delete empty group", "Șterge grupul gol"),
    (
        "Group created; v2 files keep it only once it has an entry",
        "Grup creat; fișierele v2 îl păstrează doar după ce are o intrare",
    ),
    (
        "v2 files can't store empty groups: add an entry to keep it.",
        "Fișierele v2 nu pot păstra grupuri goale: adăugați o intrare pentru a-l păstra.",
    ),
    ("Collapse", "Restrânge"),
    ("Expires", "Expiră"),
    ("YYYY-MM-DD (optional)", "AAAA-LL-ZZ (opțional)"),
    ("Notes", "Note"),
    ("Generate", "Generează"),
    ("No password", "Fără parolă"),
    ("Save", "Salvează"),
    ("Add entry", "Adaugă intrarea"),
    ("Title is required.", "Titlul este obligatoriu."),
    ("Expiry must be a date like 2027-03-31.", "Data de expirare trebuie să fie de forma 2027-03-31."),
    (
        "Note: emoji and similar characters don't display in Password Gorilla.",
        "Notă: emoji și caracterele similare nu se afișează în Password Gorilla.",
    ),
    // generator
    ("Random", "Aleatoriu"),
    ("Passphrase", "Frază"),
    ("PIN", "PIN"),
    ("Length", "Lungime"),
    ("characters", "caractere"),
    ("words", "cuvinte"),
    ("No look‑alikes", "Fără caractere asemănătoare"),
    ("Regenerate", "Regenerează"),
    ("Use password", "Folosește parola"),
    ("Weak", "Slabă"),
    ("Fair", "Acceptabilă"),
    ("Strong", "Puternică"),
    ("Excellent", "Excelentă"),
    ("{label} · {n} bits", "{label} · {n} biți"),
    ("~{n} bits", "~{n} biți"),
    // unlock
    ("Unlock vault", "Deblocare seif"),
    ("No vault selected", "Niciun seif selectat"),
    ("Unlock", "Deblochează"),
    ("Unlocking…", "Se deblochează…"),
    ("Choose vault…", "Alege seiful…"),
    ("Open another vault…", "Deschide alt seif…"),
    // create a vault
    ("Create new vault…", "Creează un seif nou…"),
    ("Create new vault", "Creează un seif nou"),
    ("Create vault", "Creează seiful"),
    ("Creating…", "Se creează…"),
    ("Repeat master password", "Repetă parola principală"),
    ("Use at least {n} characters.", "Folosește cel puțin {n} caractere."),
    ("At least {n} characters", "Cel puțin {n} caractere"),
    ("The passwords don't match.", "Parolele nu coincid."),
    ("Couldn't create the vault: {e}", "Seiful nu a putut fi creat: {e}"),
    ("{name} already exists. Keywhere never replaces a file: choose another name.", "{name} există deja. Keywhere nu înlocuiește niciodată un fișier: alege alt nume."),
    ("If you forget the master password, the vault can't be opened. Nobody can recover it, not even Keywhere. Write it down and keep it somewhere safe.", "Dacă uiți parola principală, seiful nu mai poate fi deschis. Nimeni nu o poate recupera, nici măcar Keywhere. Noteaz-o și păstreaz-o într-un loc sigur."),
    ("Open {name}? The vault that is open now will lock first.", "Deschizi {name}? Seiful deschis acum se va bloca mai întâi."),
    ("Lock and open", "Blochează și deschide"),
    ("OK", "OK"),
    ("Undo", "Anulează"),
    ("Keywhere added itself to your app menu, so you can pin it to the task bar and open .psafe3 files with it.", "Keywhere s-a adăugat în meniul de aplicații: îl poți fixa în bara de activități și poți deschide fișiere .psafe3 cu el."),
    ("Couldn't open the file dialog: {e}", "Fereastra de alegere a fișierului nu s-a deschis: {e}"),
    ("Open vault", "Deschide seiful"),
    ("Open", "Deschide"),
    ("Caps Lock is on", "Caps Lock este activat"),
    ("built", "compilat"),
    // settings
    ("Security", "Securitate"),
    ("Auto‑lock after", "Blocare automată după"),
    ("Never", "Niciodată"),
    ("{n} min", "{n} min"),
    ("Clear clipboard after", "Golește clipboardul după"),
    ("{n} s", "{n} s"),
    ("Closing the window", "La închiderea ferestrei"),
    ("Hide, keep unlocked", "Ascunde, rămâne deblocat"),
    ("Lock and hide", "Blochează și ascunde"),
    ("Language", "Limbă"),
    ("Appearance", "Aspect"),
    ("Theme", "Temă"),
    ("System", "Sistem"),
    ("Dark", "Întunecată"),
    ("Light", "Luminoasă"),
    ("System ({lang})", "Sistem ({lang})"),
    ("Done", "Gata"),
    (
        "Settings are saved immediately and never contain secrets.",
        "Setările se salvează imediat și nu conțin niciodată secrete.",
    ),
    // updates
    ("Updates", "Actualizări"),
    ("Check for updates at startup", "Caută actualizări la pornire"),
    ("Check now", "Verifică acum"),
    ("Checking…", "Se verifică…"),
    ("Keywhere {v} is up to date.", "Keywhere {v} este la zi."),
    ("Keywhere {v} is available.", "Keywhere {v} este disponibil."),
    ("Update and restart", "Actualizează și repornește"),
    ("Download", "Descarcă"),
    ("What's new", "Noutăți"),
    ("Later", "Mai târziu"),
    ("Check GitHub for new Keywhere versions at startup? Nothing about your vaults is sent.", "Verifici pe GitHub la pornire dacă există versiuni noi Keywhere? Nu se trimite nimic despre seifurile tale."),
    ("Check for updates", "Caută actualizări"),
    ("Not now", "Nu acum"),
    ("Downloading Keywhere {v}… {p}%", "Se descarcă Keywhere {v}… {p}%"),
    ("Couldn't reach GitHub. Check your connection.", "GitHub nu răspunde. Verifică conexiunea."),
    ("The update failed verification and was not installed: {e}", "Actualizarea nu a trecut verificarea și nu a fost instalată: {e}"),
    ("Couldn't install the update: {e}", "Actualizarea nu a putut fi instalată: {e}"),
    ("Checks ask GitHub for the latest release. Updates are verified with Keywhere's signing key before they are installed. Nothing about your vaults is sent.", "Verificarea întreabă GitHub care este ultima versiune. Actualizările sunt verificate cu cheia de semnare Keywhere înainte de instalare. Nu se trimite nimic despre seifurile tale."),
    ("The built-in update key is unreadable.", "Cheia de actualizare inclusă nu poate fi citită."),
    ("The update signature is malformed.", "Semnătura actualizării este coruptă."),
    ("The update information is not signed by Keywhere.", "Informațiile despre actualizare nu sunt semnate de Keywhere."),
    ("The update information is malformed.", "Informațiile despre actualizare sunt corupte."),
    ("The update information has no valid version.", "Informațiile despre actualizare nu au o versiune validă."),
    ("The update information links outside Keywhere.", "Informațiile despre actualizare trimit în afara Keywhere."),
    ("The update download is not a Keywhere release file.", "Fișierul descărcat nu face parte dintr-o versiune Keywhere."),
    ("The download is larger than announced.", "Fișierul descărcat este mai mare decât cel anunțat."),
    ("The download doesn't match the signed checksum.", "Fișierul descărcat nu corespunde sumei de control semnate."),
    // date picker
    ("In {n} days", "Peste {n} zile"),
    ("In 1 year", "Peste 1 an"),
    ("Clear", "Șterge"),
    ("Mo", "Lu"),
    ("Tu", "Ma"),
    ("We", "Mi"),
    ("Th", "Jo"),
    ("Fr", "Vi"),
    ("Sa", "Sâ"),
    ("Su", "Du"),
    ("January", "ianuarie"),
    ("February", "februarie"),
    ("March", "martie"),
    ("April", "aprilie"),
    ("June", "iunie"),
    ("July", "iulie"),
    ("August", "august"),
    ("September", "septembrie"),
    ("October", "octombrie"),
    ("November", "noiembrie"),
    ("December", "decembrie"),
    // months (short)
    ("Jan", "ian."),
    ("Feb", "feb."),
    ("Mar", "mar."),
    ("Apr", "apr."),
    ("May", "mai"),
    ("Jun", "iun."),
    ("Jul", "iul."),
    ("Aug", "aug."),
    ("Sep", "sept."),
    ("Oct", "oct."),
    ("Nov", "nov."),
    ("Dec", "dec."),
];

/// Tests that switch or depend on the global language hold this, so they
/// don't race each other.
#[cfg(test)]
pub(crate) static TEST_LANG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_fallback() {
        let _g = TEST_LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_current(Lang::Ro);
        assert_eq!(t("Search"), "Caută");
        assert_eq!(t("Not in the table"), "Not in the table");
        assert_eq!(tf("{n} entries", &[("n", "3")]), "3 intrări");
        set_current(Lang::En);
        assert_eq!(tf("{n} entries", &[("n", "3")]), "3 entries");
    }

    #[test]
    fn no_duplicate_keys() {
        let mut keys: Vec<&str> = TABLE.iter().map(|(k, _)| *k).collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(n, keys.len());
    }
}
