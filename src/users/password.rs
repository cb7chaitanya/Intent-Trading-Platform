const MIN_LENGTH: usize = 12;

const COMMON_PASSWORDS: &[&str] = &[
    "password1234", "123456789012", "qwertyuiopas", "abcdefghijkl",
    "letmein12345", "welcome12345", "administrator", "changeme1234",
    "iloveyou1234", "trustno1test", "passw0rd1234", "p@ssword1234",
    "admin1234567", "master123456", "dragon123456", "monkey123456",
    "shadow123456", "sunshine1234", "princess1234", "football1234",
    "charlie12345", "michael12345", "password!234", "1234567890ab",
    "qwerty123456", "abc123456789",
];

/// Validate password strength. Returns a list of all violations found.
pub fn validate(password: &str, email: &str) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if password.len() < MIN_LENGTH {
        errors.push(format!("Must be at least {MIN_LENGTH} characters"));
    }

    if !password.chars().any(|c| c.is_uppercase()) {
        errors.push("Must contain at least 1 uppercase letter".into());
    }

    if !password.chars().any(|c| c.is_lowercase()) {
        errors.push("Must contain at least 1 lowercase letter".into());
    }

    if !password.chars().any(|c| c.is_ascii_digit()) {
        errors.push("Must contain at least 1 number".into());
    }

    if !password.chars().any(|c| !c.is_alphanumeric()) {
        errors.push("Must contain at least 1 special character".into());
    }

    // Check if password contains email or username
    let email_lower = email.to_lowercase();
    let password_lower = password.to_lowercase();
    let local_part = email_lower.split('@').next().unwrap_or("");

    if !local_part.is_empty() && local_part.len() >= 3 && password_lower.contains(local_part) {
        errors.push("Must not contain your email or username".into());
    }

    if COMMON_PASSWORDS.contains(&password_lower.as_str()) {
        errors.push("This password is too common".into());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_password_passes() {
        assert!(validate("C0mpl3x!Pass#", "user@test.com").is_ok());
    }

    #[test]
    fn too_short() {
        let result = validate("Ab1!", "u@t.co");
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("12 characters")));
    }

    #[test]
    fn missing_uppercase() {
        let result = validate("complex!pass1#", "u@t.co");
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("uppercase")));
    }

    #[test]
    fn missing_lowercase() {
        let result = validate("COMPLEX!PASS1#", "u@t.co");
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("lowercase")));
    }

    #[test]
    fn missing_number() {
        let result = validate("Complex!Pass!#", "u@t.co");
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("number")));
    }

    #[test]
    fn missing_special() {
        let result = validate("ComplexPass123", "u@t.co");
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("special")));
    }

    #[test]
    fn contains_email() {
        let result = validate("P@ss!john1234", "john@test.com");
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("email")));
    }

    #[test]
    fn common_password() {
        let result = validate("password1234", "u@t.co");
        assert!(result.is_err());
        assert!(result.unwrap_err().iter().any(|e| e.contains("common")));
    }

    #[test]
    fn reports_all_violations() {
        let result = validate("abc", "u@t.co");
        assert!(result.unwrap_err().len() >= 3);
    }
}
