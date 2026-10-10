//! Files that paired devices offered to this one: checking an offer from the
//! network, and making its file name safe to show and to save as.

use super::offers::FileOfferInfo;

const MAX_NAME_BYTES: usize = 200;
const MAX_MIME_CHARS: usize = 100;
const FALLBACK_NAME: &str = "file";
const FALLBACK_MIME: &str = "application/octet-stream";
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Check an offer from the network and normalize what's safe to keep.
pub fn validate_offer(info: FileOfferInfo) -> Result<FileOfferInfo, String> {
    uuid::Uuid::parse_str(&info.offer_id).map_err(|_| "offer id isn't a UUID".to_string())?;
    let is_sha256 = info.sha256.len() == 64
        && info
            .sha256
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !is_sha256 {
        return Err("offer hash isn't a SHA-256".into());
    }

    let is_plain_mime = !info.mime.is_empty()
        && info.mime.len() <= MAX_MIME_CHARS
        && info.mime.chars().all(|c| c.is_ascii_graphic());
    Ok(FileOfferInfo {
        name: clean_file_name(&info.name),
        mime: if is_plain_mime {
            info.mime
        } else {
            FALLBACK_MIME.into()
        },
        ..info
    })
}

/// A file name from another device, made safe to show and to save as: only
/// its last path part, nothing that can climb out of a folder or that
/// Windows refuses, and not too long.
pub fn clean_file_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    // Windows drops trailing dots and spaces, which can turn a name into
    // something else entirely.
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return FALLBACK_NAME.into();
    }

    let stem = cleaned.split('.').next().unwrap_or("");
    let name = if WINDOWS_RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    };
    truncate_keeping_extension(&name)
}

fn truncate_keeping_extension(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if name.len() - dot <= 16 => name.split_at(dot),
        _ => (name, ""),
    };
    let mut budget = MAX_NAME_BYTES - extension.len();
    while !stem.is_char_boundary(budget) {
        budget -= 1;
    }
    format!("{}{extension}", &stem[..budget])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OFFER: &str = "33333333-3333-4333-8333-333333333333";

    fn info() -> FileOfferInfo {
        FileOfferInfo {
            offer_id: OFFER.into(),
            name: "report.pdf".into(),
            size: 2048,
            mime: "application/pdf".into(),
            sha256: SHA.into(),
        }
    }

    // ---- file names ----

    #[test]
    fn plain_name_is_kept() {
        assert_eq!(clean_file_name("report.pdf"), "report.pdf");
    }

    #[test]
    fn unicode_name_is_kept() {
        assert_eq!(clean_file_name("café menú.pdf"), "café menú.pdf");
    }

    #[test]
    fn path_climbing_out_keeps_only_the_last_part() {
        assert_eq!(clean_file_name("../../.bashrc"), ".bashrc");
    }

    #[test]
    fn windows_path_keeps_only_the_last_part() {
        assert_eq!(clean_file_name("C:\\Users\\me\\evil.exe"), "evil.exe");
    }

    #[test]
    fn dot_names_fall_back_to_a_safe_name() {
        assert_eq!(
            (clean_file_name(".."), clean_file_name(".")),
            ("file".into(), "file".into())
        );
    }

    #[test]
    fn empty_name_falls_back_to_a_safe_name() {
        assert_eq!(clean_file_name(""), "file");
    }

    #[test]
    fn control_characters_are_removed() {
        assert_eq!(clean_file_name("bad\u{0}na\nme.txt"), "badname.txt");
    }

    #[test]
    fn characters_windows_forbids_are_replaced() {
        assert_eq!(clean_file_name("a<b>c:d\"e|f?g*.txt"), "a_b_c_d_e_f_g_.txt");
    }

    #[test]
    fn trailing_dots_and_spaces_are_trimmed() {
        assert_eq!(clean_file_name("report.pdf. . "), "report.pdf");
    }

    #[test]
    fn windows_reserved_names_are_prefixed() {
        assert_eq!(
            (clean_file_name("CON.txt"), clean_file_name("nul")),
            ("_CON.txt".into(), "_nul".into())
        );
    }

    #[test]
    fn long_names_are_cut_but_keep_their_extension() {
        let cleaned = clean_file_name(&format!("{}.pdf", "a".repeat(500)));
        assert!(cleaned.len() <= MAX_NAME_BYTES && cleaned.ends_with(".pdf"));
    }

    #[test]
    fn long_unicode_names_are_cut_on_a_character_boundary() {
        // Would panic if cut mid-character.
        let cleaned = clean_file_name(&format!("{}.pdf", "é".repeat(300)));
        assert!(cleaned.ends_with(".pdf"));
    }

    // ---- validating offers ----

    #[test]
    fn valid_offer_passes() {
        assert_eq!(validate_offer(info()), Ok(info()));
    }

    #[test]
    fn offer_name_is_cleaned() {
        let offer = FileOfferInfo {
            name: "../../etc/passwd".into(),
            ..info()
        };
        assert_eq!(validate_offer(offer).unwrap().name, "passwd");
    }

    #[test]
    fn offer_with_a_non_uuid_id_is_rejected() {
        let offer = FileOfferInfo {
            offer_id: "../../x".into(),
            ..info()
        };
        assert!(validate_offer(offer).is_err());
    }

    #[test]
    fn offer_with_a_bad_hash_is_rejected() {
        let offer = FileOfferInfo {
            sha256: "not-a-hash".into(),
            ..info()
        };
        assert!(validate_offer(offer).is_err());
    }

    #[test]
    fn offer_with_an_uppercase_hash_is_rejected() {
        let offer = FileOfferInfo {
            sha256: SHA.to_uppercase(),
            ..info()
        };
        assert!(validate_offer(offer).is_err());
    }

    #[test]
    fn odd_mime_type_falls_back_to_binary() {
        let offer = FileOfferInfo {
            mime: "text/html\n<script>".into(),
            ..info()
        };
        assert_eq!(validate_offer(offer).unwrap().mime, FALLBACK_MIME);
    }
}
