// Include generated translations

include!(concat!(env!("OUT_DIR"), "/translations.rs"));

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "lang_en")]
    fn test_english_key() {
        assert_eq!(tr!("words__important"), "Important");
    }
}
