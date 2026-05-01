use unicode_normalization::UnicodeNormalization;

pub struct TextCleaner;

impl TextCleaner {
    /// Comprehensive cleaning pipeline inspired by LunaTranslator.
    pub fn clean(text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }

        // 1. Unicode Normalization (NFKC)
        let normalized: String = text.nfkc().collect();

        // 2. Process each line individually, PRESERVING line count.
        let lines: Vec<String> = normalized
            .lines()
            .map(|l| Self::process_single_line(l.trim()))
            .collect();

        // NOTE: We no longer deduplicate adjacent lines globally here 
        // because it breaks the 1-to-1 mapping with OCR boxes.
        // The AI or the UI logic should handle spatial merging if needed.

        lines.join("\n")
    }

    fn process_single_line(line: &str) -> String {
        let mut s = line.to_string();

        // a) Character Repetition Collapse
        s = Self::collapse_repeated_chars(&s);

        // b) Phrase Cycle Detection (ABCABC -> ABC)
        s = Self::collapse_repeated_phrases(&s);

        // c) Stuttering Filter (H-H-Hello -> Hello)
        s = Self::filter_stuttering(&s);

        s
    }

    fn collapse_repeated_chars(s: &str) -> String {
        if s.len() < 2 { return s.to_string(); }
        let chars: Vec<char> = s.chars().collect();
        let mut result = String::with_capacity(s.len());
        
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let mut count = 1;
            while i + count < chars.len() && chars[i + count] == c {
                count += 1;
            }

            // In manga/games:
            // - "..." -> "..." (Keep up to 3)
            // - "!!!" -> "!!!" (Keep up to 3)
            // - "AAAAA" -> "A" (Collapse if more than 2)
            // - "LL" -> "LL" (Keep double letters as they are common in English/Thai)
            
            let limit = if c == '.' || c == '!' || c == '?' || c == '。' || c == '！' || c == '？' || c == '…' {
                3
            } else if c.is_alphanumeric() {
                if count >= 3 { 1 } else { count } // Only collapse if 3+ times
            } else {
                1
            };

            for _ in 0..count.min(limit) {
                result.push(c);
            }
            i += count;
        }
        result
    }

    fn collapse_repeated_phrases(s: &str) -> String {
        if s.len() < 4 { return s.to_string(); }
        
        let result = s.to_string();
        
        // Try different window sizes for cycles (2 to length/2)
        let chars: Vec<char> = result.chars().collect();
        let len = chars.len();
        
        for win_size in 2..=(len / 2) {
            let chunk1 = &chars[0..win_size];
            let chunk2 = &chars[win_size..win_size*2];
            
            if chunk1 == chunk2 {
                let mut matches = 2;
                while (matches + 1) * win_size <= len {
                    let next_chunk = &chars[matches * win_size .. (matches + 1) * win_size];
                    if next_chunk == chunk1 {
                        matches += 1;
                    } else {
                        break;
                    }
                }
                
                if matches >= 2 && matches * win_size >= len - 1 {
                    return chunk1.iter().collect();
                }
            }
        }
        
        result
    }

    fn filter_stuttering(s: &str) -> String {
        let words: Vec<&str> = s.split_whitespace().collect();
        let mut result_words = Vec::new();
        
        let mut i = 0;
        while i < words.len() {
            let current = words[i];
            if i + 1 < words.len() {
                let next = words[i+1];
                let c_clean = current.trim_end_matches('-');
                if current.ends_with('-') && !c_clean.is_empty() && next.to_lowercase().starts_with(&c_clean.to_lowercase()) {
                    i += 1;
                    continue;
                }
            }
            result_words.push(current);
            i += 1;
        }
        
        result_words.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_collapse() {
        assert_eq!(TextCleaner::clean("AAAAABBB"), "AB");
        assert_eq!(TextCleaner::clean("Hellooooo"), "Hello");
        assert_eq!(TextCleaner::clean("Wait!!!!!!"), "Wait!!!");
    }

    #[test]
    fn test_cycle_collapse() {
        assert_eq!(TextCleaner::clean("ABCABCABC"), "ABC");
        assert_eq!(TextCleaner::clean("ในที่สุดในที่สุด"), "ในที่สุด");
    }

    #[test]
    fn test_line_dedup() {
        let input = "ในที่สุด...\nในที่สุดก็กลับบ้านได้";
        assert_eq!(TextCleaner::clean(input), "ในที่สุดก็กลับบ้านได้");
    }
}
