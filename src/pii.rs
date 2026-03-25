/// Mask PII for logging. Never log raw emails or phone numbers.

pub fn mask_email(email: &str) -> String {
    if let Some(at) = email.find('@') {
        let local = &email[..at];
        let domain = &email[at..];
        if local.len() <= 2 {
            format!("**{}", domain)
        } else {
            format!("{}***{}", &local[..1], domain)
        }
    } else {
        "***".to_string()
    }
}

pub fn mask_phone(phone: &str) -> String {
    let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        "***".to_string()
    } else {
        let last4 = &digits[digits.len() - 4..];
        format!("***{}", last4)
    }
}

pub fn mask_recipient(channel: &str, recipient: &str) -> String {
    match channel {
        "email" => mask_email(recipient),
        "sms" => mask_phone(recipient),
        "push" => {
            if recipient.len() > 8 {
                format!("{}...", &recipient[..8])
            } else {
                "***".to_string()
            }
        }
        _ => {
            // subscriber_id — not PII, show as-is
            recipient.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_email() {
        assert_eq!(mask_email("jean.dupont@example.com"), "j***@example.com");
        assert_eq!(mask_email("ab@x.com"), "**@x.com");
    }

    #[test]
    fn test_mask_phone() {
        assert_eq!(mask_phone("+33612345678"), "***5678");
        assert_eq!(mask_phone("06 12 34 56 78"), "***5678");
    }
}
