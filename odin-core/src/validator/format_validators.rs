/// Validates a string value against a named format.
///
/// Returns:
/// - `Some(Ok(()))` if the value is valid for the format
/// - `Some(Err(message))` if the value is invalid for the format
/// - `None` if the format name is not recognized
pub fn validate_format(value: &str, format: &str) -> Option<Result<(), String>> {
    // Allocation-free dispatch: case-insensitive compare against each name
    // without lowercasing `format` into a heap String.
    let eq = |name: &str| format.eq_ignore_ascii_case(name);

    if eq("email") {
        Some(validate_email(value))
    } else if eq("url") {
        Some(validate_url(value))
    } else if eq("uri") {
        Some(validate_uri(value))
    } else if eq("uuid") {
        Some(validate_uuid(value))
    } else if eq("ssn") {
        Some(validate_ssn(value))
    } else if eq("vin") {
        Some(validate_vin(value))
    } else if eq("phone") {
        Some(validate_phone(value))
    } else if eq("zip") {
        Some(validate_zip(value))
    } else if eq("hostname") {
        Some(validate_hostname(value))
    } else if eq("ipv4") {
        Some(validate_ipv4(value))
    } else if eq("ipv6") {
        Some(validate_ipv6(value))
    } else if eq("datetime") || eq("date-time") {
        Some(validate_datetime(value))
    } else if eq("creditcard") || eq("credit-card") {
        Some(validate_creditcard(value))
    } else if eq("iban") {
        Some(validate_iban(value))
    } else if eq("bic") || eq("swift") {
        Some(validate_bic(value))
    } else if eq("routing") {
        Some(validate_routing(value))
    } else if eq("cusip") {
        Some(validate_cusip(value))
    } else if eq("isin") {
        Some(validate_isin(value))
    } else if eq("lei") {
        Some(validate_lei(value))
    } else if eq("npi") {
        Some(validate_npi(value))
    } else if eq("dea") {
        Some(validate_dea(value))
    } else if eq("imei") {
        Some(validate_imei(value))
    } else if eq("iccid") {
        Some(validate_iccid(value))
    } else if eq("date-iso") {
        {
            let b = value.as_bytes();
            let valid = b.len() == 10
                && b[0..4].iter().all(|c| c.is_ascii_digit())
                && b[4] == b'-'
                && b[5..7].iter().all(|c| c.is_ascii_digit())
                && b[7] == b'-'
                && b[8..10].iter().all(|c| c.is_ascii_digit());
            Some(if valid {
                Ok(())
            } else {
                Err(format!("Value '{}' does not match date-iso format (YYYY-MM-DD)", value))
            })
        }
    } else if eq("naic") {
        Some(validate_naic(value))
    } else if eq("fein") {
        Some(validate_fein(value))
    } else if eq("currency-code") {
        Some(validate_currency_code(value))
    } else if eq("country-alpha2") {
        Some(validate_country_alpha2(value))
    } else if eq("country-alpha3") {
        Some(validate_country_alpha3(value))
    } else if eq("state-us") {
        Some(validate_state_us(value))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Individual validators (all private)
// ---------------------------------------------------------------------------

fn validate_email(value: &str) -> Result<(), String> {
    // Must contain exactly one '@'
    let mut at_pos = None;
    let mut at_count = 0;
    for (i, ch) in value.chars().enumerate() {
        if ch == '@' {
            at_count += 1;
            at_pos = Some(i);
        }
    }
    if at_count != 1 {
        return Err("Invalid email format".to_string());
    }
    // Safe: at_count == 1 guarantees at_pos was set.
    let Some(at_pos) = at_pos else {
        return Err("Invalid email format".to_string());
    };

    // Local part must be non-empty
    let local = &value[..at_pos];
    if local.is_empty() {
        return Err("Invalid email format".to_string());
    }

    // Domain must contain a dot
    let domain = &value[at_pos + 1..];
    if domain.is_empty() || !domain.contains('.') {
        return Err("Invalid email format".to_string());
    }

    Ok(())
}

fn validate_url(value: &str) -> Result<(), String> {
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(())
    } else {
        Err("Invalid URL format".to_string())
    }
}

fn validate_uri(value: &str) -> Result<(), String> {
    // scheme ":" ... — scheme: alpha followed by alpha/digit/+/-/.
    let bytes = value.as_bytes();
    let Some(colon) = value.find(':') else {
        return Err("Invalid URI format".to_string());
    };
    if colon == 0 || !bytes[0].is_ascii_alphabetic() {
        return Err("Invalid URI format".to_string());
    }
    for &b in &bytes[1..colon] {
        if !(b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.') {
            return Err("Invalid URI format".to_string());
        }
    }
    // Remainder must contain no whitespace.
    for &b in &bytes[colon + 1..] {
        if b.is_ascii_whitespace() {
            return Err("Invalid URI format".to_string());
        }
    }
    Ok(())
}

fn validate_hostname(value: &str) -> Result<(), String> {
    // Dot-separated labels; each label alphanumeric, may contain hyphens but not
    // start or end with one. 1-63 chars per label.
    if value.is_empty() {
        return Err("Invalid hostname format".to_string());
    }
    for label in value.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err("Invalid hostname format".to_string());
        }
        let bytes = label.as_bytes();
        if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
            return Err("Invalid hostname format".to_string());
        }
        for &b in bytes {
            if !(b.is_ascii_alphanumeric() || b == b'-') {
                return Err("Invalid hostname format".to_string());
            }
        }
    }
    Ok(())
}

fn validate_datetime(value: &str) -> Result<(), String> {
    // ISO 8601: YYYY-MM-DDTHH:MM:SS with optional trailing content.
    let b = value.as_bytes();
    let valid = b.len() >= 19
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'T'
        && b[11..13].iter().all(u8::is_ascii_digit)
        && b[13] == b':'
        && b[14..16].iter().all(u8::is_ascii_digit)
        && b[16] == b':'
        && b[17..19].iter().all(u8::is_ascii_digit);
    if valid {
        Ok(())
    } else {
        Err(format!("Value '{}' does not match datetime format (ISO 8601)", value))
    }
}

fn validate_uuid(value: &str) -> Result<(), String> {
    // 8-4-4-4-12 hex with dashes: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return Err("Invalid UUID format".to_string());
    }

    let expected_lengths = [8, 4, 4, 4, 12];
    let mut pos = 0;
    for (group_idx, &len) in expected_lengths.iter().enumerate() {
        for _ in 0..len {
            if pos >= bytes.len() {
                return Err("Invalid UUID format".to_string());
            }
            let b = bytes[pos];
            if !is_hex_digit(b) {
                return Err("Invalid UUID format".to_string());
            }
            pos += 1;
        }
        if group_idx < 4 {
            if pos >= bytes.len() || bytes[pos] != b'-' {
                return Err("Invalid UUID format".to_string());
            }
            pos += 1;
        }
    }

    Ok(())
}

fn validate_ssn(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();

    // Count digits and capture the first three, single-pass, no allocation.
    let mut digit_count = 0usize;
    let mut first_three = [0u8; 3];
    for &b in bytes {
        if b.is_ascii_digit() {
            if digit_count < 3 {
                first_three[digit_count] = b;
            }
            digit_count += 1;
        }
    }

    if digit_count == 9 {
        // Could be ###-##-#### or #########
        let valid_format = bytes.len() == 9
            || (bytes.len() == 11 && bytes[3] == b'-' && bytes[6] == b'-');
        if !valid_format {
            return Err("Invalid SSN format".to_string());
        }
    } else {
        return Err("Invalid SSN format".to_string());
    }

    // Area code (first 3 digits) cannot be 000
    if first_three == [b'0', b'0', b'0'] {
        return Err("Invalid SSN - area code cannot be 000".to_string());
    }

    Ok(())
}

fn validate_vin(value: &str) -> Result<(), String> {
    if value.len() != 17 {
        return Err("VIN must be 17 characters".to_string());
    }

    for ch in value.chars() {
        let upper = ch.to_ascii_uppercase();
        if upper == 'I' || upper == 'O' || upper == 'Q' {
            return Err("VIN cannot contain I, O, or Q".to_string());
        }
        if !ch.is_ascii_alphanumeric() {
            return Err("VIN must be 17 characters".to_string());
        }
    }

    Ok(())
}

fn validate_phone(value: &str) -> Result<(), String> {
    // Allowed characters: digits, dashes, spaces, parens, optional leading +
    let mut digit_count = 0;
    let mut seen_plus = false;
    for (i, ch) in value.chars().enumerate() {
        match ch {
            '0'..='9' => digit_count += 1,
            '-' | ' ' | '(' | ')' => {}
            '+' => {
                if i != 0 || seen_plus {
                    return Err("Invalid phone format".to_string());
                }
                seen_plus = true;
            }
            _ => return Err("Invalid phone format".to_string()),
        }
    }

    if digit_count < 7 {
        return Err("Invalid phone format".to_string());
    }

    Ok(())
}

fn validate_zip(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();

    // 5 digits
    if bytes.len() == 5 {
        for &b in bytes {
            if !b.is_ascii_digit() {
                return Err("Invalid ZIP format".to_string());
            }
        }
        return Ok(());
    }

    // 5 digits, dash, 4 digits
    if bytes.len() == 10 {
        for &byte in &bytes[..5] {
            if !byte.is_ascii_digit() {
                return Err("Invalid ZIP format".to_string());
            }
        }
        if bytes[5] != b'-' {
            return Err("Invalid ZIP format".to_string());
        }
        for &byte in &bytes[6..10] {
            if !byte.is_ascii_digit() {
                return Err("Invalid ZIP format".to_string());
            }
        }
        return Ok(());
    }

    Err("Invalid ZIP format".to_string())
}

fn validate_ipv4(value: &str) -> Result<(), String> {
    let err = || Err("Invalid IPv4 format".to_string());
    let mut part_count = 0usize;

    for part in value.split('.') {
        part_count += 1;
        if part_count > 4 {
            return err();
        }
        let bytes = part.as_bytes();
        if bytes.is_empty() || bytes.len() > 3 {
            return err();
        }
        // Must be all digits.
        let mut n: u32 = 0;
        for &b in bytes {
            if !b.is_ascii_digit() {
                return err();
            }
            n = n * 10 + u32::from(b - b'0');
        }
        // No leading zeros (except "0" itself).
        if bytes.len() > 1 && bytes[0] == b'0' {
            return err();
        }
        if n > 255 {
            return err();
        }
    }

    if part_count != 4 {
        return err();
    }

    Ok(())
}

fn validate_ipv6(value: &str) -> Result<(), String> {
    // Handle :: compressed notation
    if value.contains("::") {
        // Only one :: allowed
        // Safe: we just checked value.contains("::").
        let Some(first) = value.find("::") else {
            return Err("Invalid IPv6 format".to_string());
        };
        if value[first + 2..].contains("::") {
            return Err("Invalid IPv6 format".to_string());
        }

        let left = &value[..first];
        let right = &value[first + 2..];

        // Count and validate groups without collecting into Vecs.
        let mut total = 0usize;
        for side in [left, right] {
            if side.is_empty() {
                continue;
            }
            for group in side.split(':') {
                total += 1;
                if total > 7 {
                    return Err("Invalid IPv6 format".to_string());
                }
                if !is_valid_ipv6_group(group) {
                    return Err("Invalid IPv6 format".to_string());
                }
            }
        }

        Ok(())
    } else {
        // Full notation: exactly 8 groups
        let mut count = 0usize;
        for group in value.split(':') {
            count += 1;
            if count > 8 {
                return Err("Invalid IPv6 format".to_string());
            }
            if !is_valid_ipv6_group(group) {
                return Err("Invalid IPv6 format".to_string());
            }
        }
        if count != 8 {
            return Err("Invalid IPv6 format".to_string());
        }

        Ok(())
    }
}

fn is_valid_ipv6_group(group: &str) -> bool {
    if group.is_empty() || group.len() > 4 {
        return false;
    }
    for b in group.bytes() {
        if !is_hex_digit(b) {
            return false;
        }
    }
    true
}

fn validate_creditcard(value: &str) -> Result<(), String> {
    // Single pass: reject invalid chars, count digits, and accumulate the Luhn
    // sum from the right by walking bytes in reverse — no digit Vec.
    let mut digit_count = 0usize;
    let mut sum: u32 = 0;
    for &b in value.as_bytes().iter().rev() {
        if b.is_ascii_digit() {
            let mut n = u32::from(b - b'0');
            if digit_count % 2 == 1 {
                n *= 2;
                if n > 9 {
                    n -= 9;
                }
            }
            sum += n;
            digit_count += 1;
        } else if b != b' ' && b != b'-' {
            return Err("Invalid credit card format".to_string());
        }
    }

    if digit_count < 13 || digit_count > 19 {
        return Err("Invalid credit card format".to_string());
    }

    if sum % 10 != 0 {
        return Err("Invalid credit card checksum".to_string());
    }

    Ok(())
}

#[cfg(test)]
fn luhn_check(digits: &[u8]) -> bool {
    let mut sum: u32 = 0;
    for (i, &d) in digits.iter().rev().enumerate() {
        let mut n = u32::from(d);
        if i % 2 == 1 {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        sum += n;
    }
    sum % 10 == 0
}

fn validate_iban(value: &str) -> Result<(), String> {
    // 2 letters, 2 digits, then 4-30 alphanumerics.
    let b = value.as_bytes();
    if b.len() < 8 || b.len() > 34 {
        return Err("Invalid IBAN format".to_string());
    }
    if !b[0].is_ascii_alphabetic() || !b[1].is_ascii_alphabetic() {
        return Err("Invalid IBAN format".to_string());
    }
    if !b[2].is_ascii_digit() || !b[3].is_ascii_digit() {
        return Err("Invalid IBAN format".to_string());
    }
    for &byte in &b[4..] {
        if !byte.is_ascii_alphanumeric() {
            return Err("Invalid IBAN format".to_string());
        }
    }
    Ok(())
}

fn validate_bic(value: &str) -> Result<(), String> {
    // 6 letters, 2 alphanumerics, optional 3 alphanumeric branch code (8 or 11).
    let b = value.as_bytes();
    if b.len() != 8 && b.len() != 11 {
        return Err("Invalid BIC format".to_string());
    }
    for &byte in &b[0..6] {
        if !byte.is_ascii_alphabetic() {
            return Err("Invalid BIC format".to_string());
        }
    }
    for &byte in &b[6..] {
        if !byte.is_ascii_alphanumeric() {
            return Err("Invalid BIC format".to_string());
        }
    }
    Ok(())
}

fn validate_routing(value: &str) -> Result<(), String> {
    if value.len() == 9 && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err("Invalid routing number format".to_string())
    }
}

fn validate_cusip(value: &str) -> Result<(), String> {
    if value.len() == 9 && value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err("Invalid CUSIP format".to_string())
    }
}

fn validate_isin(value: &str) -> Result<(), String> {
    // 2 letters, 9 alphanumerics, 1 digit.
    let b = value.as_bytes();
    if b.len() != 12 {
        return Err("Invalid ISIN format".to_string());
    }
    if !b[0].is_ascii_alphabetic() || !b[1].is_ascii_alphabetic() {
        return Err("Invalid ISIN format".to_string());
    }
    for &byte in &b[2..11] {
        if !byte.is_ascii_alphanumeric() {
            return Err("Invalid ISIN format".to_string());
        }
    }
    if !b[11].is_ascii_digit() {
        return Err("Invalid ISIN format".to_string());
    }
    Ok(())
}

fn validate_lei(value: &str) -> Result<(), String> {
    if value.len() == 20 && value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err("Invalid LEI format".to_string())
    }
}

fn validate_npi(value: &str) -> Result<(), String> {
    if value.len() == 10 && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err("Invalid NPI format".to_string())
    }
}

fn validate_dea(value: &str) -> Result<(), String> {
    // 2 letters followed by 7 digits.
    let b = value.as_bytes();
    if b.len() != 9 {
        return Err("Invalid DEA format".to_string());
    }
    if !b[0].is_ascii_alphabetic() || !b[1].is_ascii_alphabetic() {
        return Err("Invalid DEA format".to_string());
    }
    for &byte in &b[2..] {
        if !byte.is_ascii_digit() {
            return Err("Invalid DEA format".to_string());
        }
    }
    Ok(())
}

fn validate_imei(value: &str) -> Result<(), String> {
    if value.len() == 15 && value.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err("Invalid IMEI format".to_string())
    }
}

fn validate_iccid(value: &str) -> Result<(), String> {
    if (value.len() == 19 || value.len() == 20)
        && value.bytes().all(|b| b.is_ascii_digit())
    {
        Ok(())
    } else {
        Err("Invalid ICCID format".to_string())
    }
}

fn validate_naic(value: &str) -> Result<(), String> {
    if value.len() != 5 {
        return Err("Invalid NAIC code format".to_string());
    }
    for b in value.bytes() {
        if !b.is_ascii_digit() {
            return Err("Invalid NAIC code format".to_string());
        }
    }
    Ok(())
}

fn validate_fein(value: &str) -> Result<(), String> {
    // ##-#######
    let bytes = value.as_bytes();
    if bytes.len() != 10 {
        return Err("Invalid FEIN format".to_string());
    }
    if !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || bytes[2] != b'-'
    {
        return Err("Invalid FEIN format".to_string());
    }
    for &byte in &bytes[3..10] {
        if !byte.is_ascii_digit() {
            return Err("Invalid FEIN format".to_string());
        }
    }
    Ok(())
}

fn validate_currency_code(value: &str) -> Result<(), String> {
    if value.len() != 3 {
        return Err("Unknown currency code".to_string());
    }
    for b in value.bytes() {
        if !b.is_ascii_uppercase() {
            return Err("Unknown currency code".to_string());
        }
    }

    const CODES: &[&str] = &[
        "AED", "ARS", "AUD", "BDT", "BGN", "BHD", "BRL", "CAD", "CHF", "CLP",
        "CNY", "COP", "CZK", "DKK", "EGP", "EUR", "GBP", "GHS", "HKD", "HRK",
        "HUF", "IDR", "ILS", "INR", "ISK", "JOD", "JPY", "KES", "KRW", "KWD",
        "LBP", "MAD", "MXN", "MYR", "NGN", "NOK", "NZD", "OMR", "PEN", "PHP",
        "PKR", "PLN", "QAR", "RON", "RUB", "SAR", "SEK", "SGD", "THB", "TRY",
        "TWD", "TZS", "UAH", "UGX", "USD", "VND", "ZAR",
    ];

    if CODES.binary_search(&value).is_err() {
        return Err("Unknown currency code".to_string());
    }

    Ok(())
}

fn validate_country_alpha2(value: &str) -> Result<(), String> {
    if value.len() != 2 {
        return Err("Invalid country code".to_string());
    }
    for b in value.bytes() {
        if !b.is_ascii_uppercase() {
            return Err("Invalid country code".to_string());
        }
    }

    const CODES: &[&str] = &[
        "AE", "AR", "AT", "AU", "BD", "BE", "BG", "BH", "BR", "CA",
        "CH", "CL", "CN", "CO", "CY", "CZ", "DE", "DK", "EG", "ES",
        "FI", "FR", "GB", "GH", "GR", "HK", "HR", "HU", "ID", "IE",
        "IL", "IN", "IS", "IT", "JO", "JP", "KE", "KR", "KW", "LB",
        "MA", "MX", "MY", "NG", "NL", "NO", "NZ", "OM", "PE", "PH",
        "PK", "PL", "PT", "QA", "RO", "RU", "SA", "SE", "SG", "TH",
        "TR", "TW", "TZ", "UA", "UG", "US", "VN", "ZA",
    ];

    if CODES.binary_search(&value).is_err() {
        return Err("Invalid country code".to_string());
    }

    Ok(())
}

fn validate_country_alpha3(value: &str) -> Result<(), String> {
    if value.len() != 3 {
        return Err("Invalid country code".to_string());
    }
    for b in value.bytes() {
        if !b.is_ascii_uppercase() {
            return Err("Invalid country code".to_string());
        }
    }

    const CODES: &[&str] = &[
        "ARE", "ARG", "AUS", "AUT", "BEL", "BGD", "BGR", "BHR", "BRA", "CAN",
        "CHE", "CHL", "CHN", "COL", "CYP", "CZE", "DEU", "DNK", "EGY", "ESP",
        "FIN", "FRA", "GBR", "GHA", "GRC", "HKG", "HRV", "HUN", "IDN", "IND",
        "IRL", "ISL", "ISR", "ITA", "JOR", "JPN", "KEN", "KOR", "KWT", "LBN",
        "MAR", "MEX", "MYS", "NGA", "NLD", "NOR", "NZL", "OMN", "PAK", "PER",
        "PHL", "POL", "PRT", "QAT", "ROU", "RUS", "SAU", "SGP", "SWE", "THA",
        "TUR", "TWN", "TZA", "UGA", "UKR", "USA", "VNM", "ZAF",
    ];

    if CODES.binary_search(&value).is_err() {
        return Err("Invalid country code".to_string());
    }

    Ok(())
}

fn validate_state_us(value: &str) -> Result<(), String> {
    if value.len() != 2 {
        return Err("Invalid US state code".to_string());
    }
    for b in value.bytes() {
        if !b.is_ascii_uppercase() {
            return Err("Invalid US state code".to_string());
        }
    }

    const CODES: &[&str] = &[
        "AK", "AL", "AR", "AS", "AZ", "CA", "CO", "CT", "DC", "DE",
        "FL", "GA", "GU", "HI", "IA", "ID", "IL", "IN", "KS", "KY",
        "LA", "MA", "MD", "ME", "MI", "MN", "MO", "MP", "MS", "MT",
        "NC", "ND", "NE", "NH", "NJ", "NM", "NV", "NY", "OH", "OK",
        "OR", "PA", "PR", "RI", "SC", "SD", "TN", "TX", "UT", "VA",
        "VI", "VT", "WA", "WI", "WV", "WY",
    ];

    if CODES.binary_search(&value).is_err() {
        return Err("Invalid US state code".to_string());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_hex_digit(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || (b'A'..=b'F').contains(&b)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Unknown format --
    #[test]
    fn unknown_format_returns_none() {
        assert!(validate_format("anything", "nonexistent").is_none());
    }

    #[test]
    fn case_insensitive_format_name() {
        assert!(validate_format("test@example.com", "EMAIL").is_some());
        assert!(validate_format("test@example.com", "Email").is_some());
    }

    // -- email --
    #[test]
    fn email_valid() {
        assert!(validate_format("user@example.com", "email").unwrap().is_ok());
    }

    #[test]
    fn email_no_at() {
        assert!(validate_format("userexample.com", "email").unwrap().is_err());
    }

    #[test]
    fn email_double_at() {
        assert!(validate_format("user@@example.com", "email").unwrap().is_err());
    }

    #[test]
    fn email_empty_local() {
        assert!(validate_format("@example.com", "email").unwrap().is_err());
    }

    #[test]
    fn email_no_dot_in_domain() {
        assert!(validate_format("user@localhost", "email").unwrap().is_err());
    }

    // -- url --
    #[test]
    fn url_http() {
        assert!(validate_format("http://example.com", "url").unwrap().is_ok());
    }

    #[test]
    fn url_https() {
        assert!(validate_format("https://example.com/path", "url").unwrap().is_ok());
    }

    #[test]
    fn url_ftp_invalid() {
        assert!(validate_format("ftp://example.com", "url").unwrap().is_err());
    }

    // -- uuid --
    #[test]
    fn uuid_valid_lowercase() {
        assert!(validate_format("550e8400-e29b-41d4-a716-446655440000", "uuid").unwrap().is_ok());
    }

    #[test]
    fn uuid_valid_uppercase() {
        assert!(validate_format("550E8400-E29B-41D4-A716-446655440000", "uuid").unwrap().is_ok());
    }

    #[test]
    fn uuid_wrong_length() {
        assert!(validate_format("550e8400-e29b-41d4-a716", "uuid").unwrap().is_err());
    }

    #[test]
    fn uuid_no_dashes() {
        assert!(validate_format("550e8400e29b41d4a716446655440000", "uuid").unwrap().is_err());
    }

    // -- ssn --
    #[test]
    fn ssn_valid_dashes() {
        assert!(validate_format("123-45-6789", "ssn").unwrap().is_ok());
    }

    #[test]
    fn ssn_valid_no_dashes() {
        assert!(validate_format("123456789", "ssn").unwrap().is_ok());
    }

    #[test]
    fn ssn_area_000() {
        let result = validate_format("000-45-6789", "ssn").unwrap();
        assert_eq!(result.unwrap_err(), "Invalid SSN - area code cannot be 000");
    }

    #[test]
    fn ssn_wrong_format() {
        assert!(validate_format("12-345-6789", "ssn").unwrap().is_err());
    }

    // -- vin --
    #[test]
    fn vin_valid() {
        assert!(validate_format("1HGBH41JXMN109186", "vin").unwrap().is_ok());
    }

    #[test]
    fn vin_too_short() {
        let result = validate_format("1HGBH41JX", "vin").unwrap();
        assert_eq!(result.unwrap_err(), "VIN must be 17 characters");
    }

    #[test]
    fn vin_contains_i() {
        let result = validate_format("1HGBH41IXMN109186", "vin").unwrap();
        assert_eq!(result.unwrap_err(), "VIN cannot contain I, O, or Q");
    }

    #[test]
    fn vin_contains_o() {
        let result = validate_format("1HGBH41OXMN109186", "vin").unwrap();
        assert_eq!(result.unwrap_err(), "VIN cannot contain I, O, or Q");
    }

    #[test]
    fn vin_contains_q() {
        let result = validate_format("1HGBH41QXMN109186", "vin").unwrap();
        assert_eq!(result.unwrap_err(), "VIN cannot contain I, O, or Q");
    }

    // -- phone --
    #[test]
    fn phone_valid_us() {
        assert!(validate_format("(555) 123-4567", "phone").unwrap().is_ok());
    }

    #[test]
    fn phone_valid_international() {
        assert!(validate_format("+1 555 123 4567", "phone").unwrap().is_ok());
    }

    #[test]
    fn phone_too_few_digits() {
        let result = validate_format("123-456", "phone").unwrap();
        assert_eq!(result.unwrap_err(), "Invalid phone format");
    }

    // -- zip --
    #[test]
    fn zip_5_digits() {
        assert!(validate_format("90210", "zip").unwrap().is_ok());
    }

    #[test]
    fn zip_5_plus_4() {
        assert!(validate_format("90210-1234", "zip").unwrap().is_ok());
    }

    #[test]
    fn zip_invalid() {
        assert!(validate_format("9021", "zip").unwrap().is_err());
    }

    // -- ipv4 --
    #[test]
    fn ipv4_valid() {
        assert!(validate_format("192.168.1.1", "ipv4").unwrap().is_ok());
    }

    #[test]
    fn ipv4_zero() {
        assert!(validate_format("0.0.0.0", "ipv4").unwrap().is_ok());
    }

    #[test]
    fn ipv4_max() {
        assert!(validate_format("255.255.255.255", "ipv4").unwrap().is_ok());
    }

    #[test]
    fn ipv4_octet_too_large() {
        assert!(validate_format("256.1.1.1", "ipv4").unwrap().is_err());
    }

    #[test]
    fn ipv4_leading_zeros() {
        assert!(validate_format("192.168.01.1", "ipv4").unwrap().is_err());
    }

    #[test]
    fn ipv4_too_few_parts() {
        assert!(validate_format("192.168.1", "ipv4").unwrap().is_err());
    }

    // -- ipv6 --
    #[test]
    fn ipv6_full() {
        assert!(validate_format("2001:0db8:85a3:0000:0000:8a2e:0370:7334", "ipv6").unwrap().is_ok());
    }

    #[test]
    fn ipv6_compressed() {
        assert!(validate_format("2001:db8::1", "ipv6").unwrap().is_ok());
    }

    #[test]
    fn ipv6_localhost() {
        assert!(validate_format("::1", "ipv6").unwrap().is_ok());
    }

    #[test]
    fn ipv6_all_zeros() {
        assert!(validate_format("::", "ipv6").unwrap().is_ok());
    }

    #[test]
    fn ipv6_double_double_colon() {
        assert!(validate_format("2001::db8::1", "ipv6").unwrap().is_err());
    }

    // -- creditcard --
    #[test]
    fn creditcard_valid_visa() {
        // 4111111111111111 passes Luhn
        assert!(validate_format("4111111111111111", "creditcard").unwrap().is_ok());
    }

    #[test]
    fn creditcard_valid_with_spaces() {
        assert!(validate_format("4111 1111 1111 1111", "creditcard").unwrap().is_ok());
    }

    #[test]
    fn creditcard_valid_with_dashes() {
        assert!(validate_format("4111-1111-1111-1111", "creditcard").unwrap().is_ok());
    }

    #[test]
    fn creditcard_too_short() {
        assert!(validate_format("411111111111", "creditcard").unwrap().is_err());
    }

    #[test]
    fn creditcard_bad_luhn() {
        let result = validate_format("4111111111111112", "creditcard").unwrap();
        assert_eq!(result.unwrap_err(), "Invalid credit card checksum");
    }

    // -- date-iso --
    #[test]
    fn date_iso_valid() {
        assert!(validate_format("2024-01-15", "date-iso").unwrap().is_ok());
    }

    #[test]
    fn date_iso_invalid_text() {
        assert!(validate_format("anything", "date-iso").unwrap().is_err());
    }

    #[test]
    fn date_iso_wrong_separator() {
        assert!(validate_format("2024/01/15", "date-iso").unwrap().is_err());
    }

    #[test]
    fn date_iso_too_short() {
        assert!(validate_format("2024-1-15", "date-iso").unwrap().is_err());
    }

    #[test]
    fn date_iso_too_long() {
        assert!(validate_format("2024-01-155", "date-iso").unwrap().is_err());
    }

    // -- naic --
    #[test]
    fn naic_valid() {
        assert!(validate_format("12345", "naic").unwrap().is_ok());
    }

    #[test]
    fn naic_too_short() {
        assert!(validate_format("1234", "naic").unwrap().is_err());
    }

    #[test]
    fn naic_letters() {
        assert!(validate_format("1234A", "naic").unwrap().is_err());
    }

    // -- fein --
    #[test]
    fn fein_valid() {
        assert!(validate_format("12-3456789", "fein").unwrap().is_ok());
    }

    #[test]
    fn fein_no_dash() {
        assert!(validate_format("123456789", "fein").unwrap().is_err());
    }

    #[test]
    fn fein_wrong_dash_position() {
        assert!(validate_format("123-456789", "fein").unwrap().is_err());
    }

    // -- currency-code --
    #[test]
    fn currency_usd() {
        assert!(validate_format("USD", "currency-code").unwrap().is_ok());
    }

    #[test]
    fn currency_eur() {
        assert!(validate_format("EUR", "currency-code").unwrap().is_ok());
    }

    #[test]
    fn currency_unknown() {
        let result = validate_format("XYZ", "currency-code").unwrap();
        assert_eq!(result.unwrap_err(), "Unknown currency code");
    }

    #[test]
    fn currency_lowercase() {
        assert!(validate_format("usd", "currency-code").unwrap().is_err());
    }

    // -- country-alpha2 --
    #[test]
    fn country_alpha2_us() {
        assert!(validate_format("US", "country-alpha2").unwrap().is_ok());
    }

    #[test]
    fn country_alpha2_unknown() {
        let result = validate_format("XX", "country-alpha2").unwrap();
        assert_eq!(result.unwrap_err(), "Invalid country code");
    }

    // -- country-alpha3 --
    #[test]
    fn country_alpha3_usa() {
        assert!(validate_format("USA", "country-alpha3").unwrap().is_ok());
    }

    #[test]
    fn country_alpha3_unknown() {
        let result = validate_format("XXX", "country-alpha3").unwrap();
        assert_eq!(result.unwrap_err(), "Invalid country code");
    }

    // -- state-us --
    #[test]
    fn state_us_ca() {
        assert!(validate_format("CA", "state-us").unwrap().is_ok());
    }

    #[test]
    fn state_us_dc() {
        assert!(validate_format("DC", "state-us").unwrap().is_ok());
    }

    #[test]
    fn state_us_pr() {
        assert!(validate_format("PR", "state-us").unwrap().is_ok());
    }

    #[test]
    fn state_us_unknown() {
        let result = validate_format("XX", "state-us").unwrap();
        assert_eq!(result.unwrap_err(), "Invalid US state code");
    }

    // ── Additional email tests ──────────────────────────────────────────

    #[test]
    fn email_with_plus() {
        assert!(validate_format("user+tag@example.com", "email").unwrap().is_ok());
    }

    #[test]
    fn email_with_dots_in_local() {
        assert!(validate_format("first.last@example.com", "email").unwrap().is_ok());
    }

    #[test]
    fn email_empty_string() {
        assert!(validate_format("", "email").unwrap().is_err());
    }

    #[test]
    fn email_no_domain() {
        assert!(validate_format("user@", "email").unwrap().is_err());
    }

    #[test]
    fn email_just_at() {
        assert!(validate_format("@", "email").unwrap().is_err());
    }

    // ── Additional URL tests ────────────────────────────────────────────

    #[test]
    fn url_with_port() {
        assert!(validate_format("http://localhost:8080", "url").unwrap().is_ok());
    }

    #[test]
    fn url_with_query_string() {
        assert!(validate_format("https://example.com/path?q=1&b=2", "url").unwrap().is_ok());
    }

    #[test]
    fn url_bare_domain() {
        assert!(validate_format("example.com", "url").unwrap().is_err());
    }

    #[test]
    fn url_empty_string() {
        assert!(validate_format("", "url").unwrap().is_err());
    }

    #[test]
    fn url_mailto_invalid() {
        assert!(validate_format("mailto:user@example.com", "url").unwrap().is_err());
    }

    // ── Additional UUID tests ───────────────────────────────────────────

    #[test]
    fn uuid_mixed_case() {
        assert!(validate_format("550e8400-E29B-41d4-A716-446655440000", "uuid").unwrap().is_ok());
    }

    #[test]
    fn uuid_empty() {
        assert!(validate_format("", "uuid").unwrap().is_err());
    }

    #[test]
    fn uuid_too_long() {
        assert!(validate_format("550e8400-e29b-41d4-a716-4466554400001", "uuid").unwrap().is_err());
    }

    #[test]
    fn uuid_with_braces() {
        assert!(validate_format("{550e8400-e29b-41d4-a716-446655440000}", "uuid").unwrap().is_err());
    }

    // ── Additional IPv4 tests ───────────────────────────────────────────

    #[test]
    fn ipv4_loopback() {
        assert!(validate_format("127.0.0.1", "ipv4").unwrap().is_ok());
    }

    #[test]
    fn ipv4_empty_string() {
        assert!(validate_format("", "ipv4").unwrap().is_err());
    }

    #[test]
    fn ipv4_too_many_parts() {
        assert!(validate_format("1.2.3.4.5", "ipv4").unwrap().is_err());
    }

    #[test]
    fn ipv4_negative_number() {
        assert!(validate_format("-1.0.0.0", "ipv4").unwrap().is_err());
    }

    #[test]
    fn ipv4_letters() {
        assert!(validate_format("abc.def.ghi.jkl", "ipv4").unwrap().is_err());
    }

    #[test]
    fn ipv4_single_zero_octet() {
        assert!(validate_format("0.0.0.0", "ipv4").unwrap().is_ok());
    }

    // ── Additional IPv6 tests ───────────────────────────────────────────

    #[test]
    fn ipv6_link_local() {
        assert!(validate_format("fe80::1", "ipv6").unwrap().is_ok());
    }

    #[test]
    fn ipv6_full_zeros() {
        assert!(validate_format("0000:0000:0000:0000:0000:0000:0000:0000", "ipv6").unwrap().is_ok());
    }

    #[test]
    fn ipv6_empty_string() {
        assert!(validate_format("", "ipv6").unwrap().is_err());
    }

    #[test]
    fn ipv6_too_many_groups() {
        assert!(validate_format("1:2:3:4:5:6:7:8:9", "ipv6").unwrap().is_err());
    }

    #[test]
    fn ipv6_invalid_hex() {
        assert!(validate_format("gggg:0000:0000:0000:0000:0000:0000:0000", "ipv6").unwrap().is_err());
    }

    #[test]
    fn ipv6_group_too_long() {
        assert!(validate_format("12345:0:0:0:0:0:0:0", "ipv6").unwrap().is_err());
    }

    // ── Additional phone tests ──────────────────────────────────────────

    #[test]
    fn phone_digits_only() {
        assert!(validate_format("5551234567", "phone").unwrap().is_ok());
    }

    #[test]
    fn phone_with_dashes() {
        assert!(validate_format("555-123-4567", "phone").unwrap().is_ok());
    }

    #[test]
    fn phone_with_letters() {
        assert!(validate_format("555-ABC-1234", "phone").unwrap().is_err());
    }

    #[test]
    fn phone_empty() {
        assert!(validate_format("", "phone").unwrap().is_err());
    }

    #[test]
    fn phone_plus_not_first() {
        assert!(validate_format("1+555-1234567", "phone").unwrap().is_err());
    }

    // ── Additional ZIP tests ────────────────────────────────────────────

    #[test]
    fn zip_with_letters() {
        assert!(validate_format("9021A", "zip").unwrap().is_err());
    }

    #[test]
    fn zip_too_long() {
        assert!(validate_format("902101234", "zip").unwrap().is_err());
    }

    #[test]
    fn zip_empty() {
        assert!(validate_format("", "zip").unwrap().is_err());
    }

    #[test]
    fn zip_plus_4_no_dash() {
        assert!(validate_format("902101234", "zip").unwrap().is_err());
    }

    // ── Additional SSN tests ────────────────────────────────────────────

    #[test]
    fn ssn_too_few_digits() {
        assert!(validate_format("12345678", "ssn").unwrap().is_err());
    }

    #[test]
    fn ssn_too_many_digits() {
        assert!(validate_format("1234567890", "ssn").unwrap().is_err());
    }

    #[test]
    fn ssn_empty() {
        assert!(validate_format("", "ssn").unwrap().is_err());
    }

    // ── Additional VIN tests ────────────────────────────────────────────

    #[test]
    fn vin_too_long() {
        let result = validate_format("1HGBH41JXMN1091861", "vin").unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn vin_lowercase_valid() {
        // VIN check is case-insensitive for I/O/Q exclusion
        assert!(validate_format("1hgbh41jxmn109186", "vin").unwrap().is_ok());
    }

    // ── Additional creditcard tests ─────────────────────────────────────

    #[test]
    fn creditcard_mastercard() {
        // 5500000000000004 passes Luhn
        assert!(validate_format("5500000000000004", "creditcard").unwrap().is_ok());
    }

    #[test]
    fn creditcard_amex() {
        // 378282246310005 passes Luhn
        assert!(validate_format("378282246310005", "creditcard").unwrap().is_ok());
    }

    #[test]
    fn creditcard_too_long() {
        assert!(validate_format("41111111111111111111", "creditcard").unwrap().is_err());
    }

    #[test]
    fn creditcard_with_letters() {
        assert!(validate_format("4111-ABCD-1111-1111", "creditcard").unwrap().is_err());
    }

    // ── Additional NAIC tests ───────────────────────────────────────────

    #[test]
    fn naic_too_long() {
        assert!(validate_format("123456", "naic").unwrap().is_err());
    }

    #[test]
    fn naic_empty() {
        assert!(validate_format("", "naic").unwrap().is_err());
    }

    // ── Additional FEIN tests ───────────────────────────────────────────

    #[test]
    fn fein_too_short() {
        assert!(validate_format("12-34567", "fein").unwrap().is_err());
    }

    #[test]
    fn fein_letters() {
        assert!(validate_format("AB-CDEFGHI", "fein").unwrap().is_err());
    }

    // ── Currency code tests ─────────────────────────────────────────────

    #[test]
    fn currency_gbp() {
        assert!(validate_format("GBP", "currency-code").unwrap().is_ok());
    }

    #[test]
    fn currency_jpy() {
        assert!(validate_format("JPY", "currency-code").unwrap().is_ok());
    }

    #[test]
    fn currency_too_short() {
        assert!(validate_format("US", "currency-code").unwrap().is_err());
    }

    #[test]
    fn currency_too_long() {
        assert!(validate_format("USDD", "currency-code").unwrap().is_err());
    }

    // ── Country code tests ──────────────────────────────────────────────

    #[test]
    fn country_alpha2_gb() {
        assert!(validate_format("GB", "country-alpha2").unwrap().is_ok());
    }

    #[test]
    fn country_alpha2_lowercase() {
        assert!(validate_format("us", "country-alpha2").unwrap().is_err());
    }

    #[test]
    fn country_alpha2_too_long() {
        assert!(validate_format("USA", "country-alpha2").unwrap().is_err());
    }

    #[test]
    fn country_alpha3_gbr() {
        assert!(validate_format("GBR", "country-alpha3").unwrap().is_ok());
    }

    #[test]
    fn country_alpha3_lowercase() {
        assert!(validate_format("usa", "country-alpha3").unwrap().is_err());
    }

    #[test]
    fn country_alpha3_too_short() {
        assert!(validate_format("US", "country-alpha3").unwrap().is_err());
    }

    // ── State US tests ──────────────────────────────────────────────────

    #[test]
    fn state_us_ny() {
        assert!(validate_format("NY", "state-us").unwrap().is_ok());
    }

    #[test]
    fn state_us_tx() {
        assert!(validate_format("TX", "state-us").unwrap().is_ok());
    }

    #[test]
    fn state_us_lowercase() {
        assert!(validate_format("ca", "state-us").unwrap().is_err());
    }

    #[test]
    fn state_us_too_long() {
        assert!(validate_format("CAL", "state-us").unwrap().is_err());
    }

    #[test]
    fn state_us_territories() {
        // GU, VI, AS, MP are included
        assert!(validate_format("GU", "state-us").unwrap().is_ok());
        assert!(validate_format("VI", "state-us").unwrap().is_ok());
        assert!(validate_format("AS", "state-us").unwrap().is_ok());
        assert!(validate_format("MP", "state-us").unwrap().is_ok());
    }

    // ── Luhn algorithm edge cases ───────────────────────────────────────

    #[test]
    fn luhn_single_zero() {
        assert!(luhn_check(&[0]));
    }

    #[test]
    fn luhn_known_valid() {
        // 79927398713
        assert!(luhn_check(&[7, 9, 9, 2, 7, 3, 9, 8, 7, 1, 3]));
    }

    #[test]
    fn luhn_known_invalid() {
        assert!(!luhn_check(&[7, 9, 9, 2, 7, 3, 9, 8, 7, 1, 4]));
    }

    // ── uri ─────────────────────────────────────────────────────────────
    #[test]
    fn uri_valid_urn() {
        assert!(validate_format("urn:isbn:0451450523", "uri").unwrap().is_ok());
    }

    #[test]
    fn uri_invalid_no_scheme() {
        assert!(validate_format("/relative/path", "uri").unwrap().is_err());
    }

    // ── hostname ────────────────────────────────────────────────────────
    #[test]
    fn hostname_valid() {
        assert!(validate_format("sub.example.co.uk", "hostname").unwrap().is_ok());
    }

    #[test]
    fn hostname_invalid_leading_hyphen() {
        assert!(validate_format("-bad.example.com", "hostname").unwrap().is_err());
    }

    #[test]
    fn hostname_invalid_underscore() {
        assert!(validate_format("bad_underscore.com", "hostname").unwrap().is_err());
    }

    // ── datetime / date-time ────────────────────────────────────────────
    #[test]
    fn datetime_valid_zulu() {
        assert!(validate_format("2024-06-15T10:30:00Z", "datetime").unwrap().is_ok());
    }

    #[test]
    fn datetime_invalid_space_separator() {
        assert!(validate_format("2024-06-15 10:30:00", "datetime").unwrap().is_err());
    }

    #[test]
    fn date_time_alias_valid() {
        assert!(validate_format("2024-06-15T10:30:00Z", "date-time").unwrap().is_ok());
    }

    #[test]
    fn date_time_alias_invalid_slashes() {
        assert!(validate_format("06/15/2024", "date-time").unwrap().is_err());
    }

    // ── credit-card alias ───────────────────────────────────────────────
    #[test]
    fn credit_card_alias_valid() {
        assert!(validate_format("4111111111111111", "credit-card").unwrap().is_ok());
    }

    #[test]
    fn credit_card_alias_bad_checksum() {
        assert!(validate_format("4111111111111112", "credit-card").unwrap().is_err());
    }

    // ── iban ────────────────────────────────────────────────────────────
    #[test]
    fn iban_valid_gb() {
        assert!(validate_format("GB82WEST12345698765432", "iban").unwrap().is_ok());
    }

    #[test]
    fn iban_invalid_no_country() {
        assert!(validate_format("1234WEST", "iban").unwrap().is_err());
    }

    // ── bic / swift ─────────────────────────────────────────────────────
    #[test]
    fn bic_valid_8() {
        assert!(validate_format("DEUTDEFF", "bic").unwrap().is_ok());
    }

    #[test]
    fn bic_valid_11() {
        assert!(validate_format("DEUTDEFF500", "bic").unwrap().is_ok());
    }

    #[test]
    fn bic_invalid_9_chars() {
        assert!(validate_format("DEUTDEFF5", "bic").unwrap().is_err());
    }

    #[test]
    fn swift_valid() {
        assert!(validate_format("BOFAUS3N", "swift").unwrap().is_ok());
    }

    #[test]
    fn swift_invalid_too_short() {
        assert!(validate_format("BOFAUS3", "swift").unwrap().is_err());
    }

    // ── routing ─────────────────────────────────────────────────────────
    #[test]
    fn routing_valid() {
        assert!(validate_format("021000021", "routing").unwrap().is_ok());
    }

    #[test]
    fn routing_invalid_8_digits() {
        assert!(validate_format("12345678", "routing").unwrap().is_err());
    }

    // ── cusip ───────────────────────────────────────────────────────────
    #[test]
    fn cusip_valid() {
        assert!(validate_format("037833100", "cusip").unwrap().is_ok());
    }

    #[test]
    fn cusip_invalid_symbol() {
        assert!(validate_format("037833$00", "cusip").unwrap().is_err());
    }

    // ── isin ────────────────────────────────────────────────────────────
    #[test]
    fn isin_valid() {
        assert!(validate_format("US0378331005", "isin").unwrap().is_ok());
    }

    #[test]
    fn isin_invalid_non_digit_check() {
        assert!(validate_format("US037833100X", "isin").unwrap().is_err());
    }

    // ── lei ─────────────────────────────────────────────────────────────
    #[test]
    fn lei_valid() {
        assert!(validate_format("529900T8BM49AURSDO55", "lei").unwrap().is_ok());
    }

    #[test]
    fn lei_invalid_symbol() {
        assert!(validate_format("529900T8BM49AURSDO5$", "lei").unwrap().is_err());
    }

    // ── npi ─────────────────────────────────────────────────────────────
    #[test]
    fn npi_valid() {
        assert!(validate_format("1234567890", "npi").unwrap().is_ok());
    }

    #[test]
    fn npi_invalid_too_long() {
        assert!(validate_format("12345678901", "npi").unwrap().is_err());
    }

    // ── dea ─────────────────────────────────────────────────────────────
    #[test]
    fn dea_valid() {
        assert!(validate_format("AB1234567", "dea").unwrap().is_ok());
    }

    #[test]
    fn dea_invalid_one_letter() {
        assert!(validate_format("A1234567", "dea").unwrap().is_err());
    }

    // ── imei ────────────────────────────────────────────────────────────
    #[test]
    fn imei_valid() {
        assert!(validate_format("490154203237518", "imei").unwrap().is_ok());
    }

    #[test]
    fn imei_invalid_too_long() {
        assert!(validate_format("4901542032375189", "imei").unwrap().is_err());
    }

    // ── iccid ───────────────────────────────────────────────────────────
    #[test]
    fn iccid_valid_19() {
        assert!(validate_format("8901234567890123456", "iccid").unwrap().is_ok());
    }

    #[test]
    fn iccid_invalid_letter() {
        assert!(validate_format("8901234567890123456X", "iccid").unwrap().is_err());
    }
}
