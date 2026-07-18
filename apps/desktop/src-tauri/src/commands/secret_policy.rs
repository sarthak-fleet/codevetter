const REDACTED: &str = "[redacted]";

pub(crate) fn is_sensitive_path(path: &str) -> bool {
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    let wrapped = format!("/{}/", lower.trim_matches('/'));
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    matches!(
        name,
        ".env"
            | ".npmrc"
            | ".pypirc"
            | ".netrc"
            | ".dockercfg"
            | "id_rsa"
            | "id_ed25519"
            | "credentials"
            | "credentials.json"
            | "secrets.yml"
            | "secrets.yaml"
            | "secrets.json"
    ) || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.ends_with(".jks")
        || name.ends_with(".keystore")
        || name == "terraform.tfstate"
        || name.starts_with("terraform.tfstate.")
        || name.contains("service-account")
        || name.contains("service_account")
        || ["/.ssh/", "/.aws/", "/.kube/", "/secrets/"]
            .iter()
            .any(|segment| wrapped.contains(segment))
}

pub(crate) fn contains_sensitive_path(value: &str) -> bool {
    is_sensitive_path(value)
        || value
            .split(|character: char| {
                character.is_whitespace()
                    || matches!(character, '`' | '\'' | '"' | ',' | ';' | '(' | ')')
            })
            .map(|token| token.trim_matches([':', '[', ']']))
            .filter(|token| !token.is_empty())
            .any(is_sensitive_path)
}

pub(crate) fn looks_like_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if [
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "sk-ant-",
        "sk-proj-",
        "github_pat_",
        "ghp_",
        "xoxb-",
        "xoxp-",
        "xapp-",
        "AIza",
        "postgres://",
        "postgresql://",
        "mongodb://",
        "mongodb+srv://",
        "mysql://",
        "redis://",
    ]
    .iter()
    .any(|marker| lower.contains(&marker.to_ascii_lowercase()))
    {
        return true;
    }
    if lower.contains("bearer ") || contains_basic_auth_url(value) {
        return true;
    }
    value
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(is_cloud_access_key)
        || contains_credential_assignment(value)
}

pub(crate) fn redact_secret_text(value: &str) -> (String, bool) {
    if looks_like_secret(value) || contains_sensitive_path(value) {
        (REDACTED.to_string(), true)
    } else {
        (value.to_string(), false)
    }
}

fn is_cloud_access_key(token: &str) -> bool {
    token.len() == 20
        && (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

fn contains_basic_auth_url(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        token
            .find("://")
            .and_then(|scheme| token[scheme + 3..].split('@').next())
            .is_some_and(|authority| authority.contains(':') && token.contains('@'))
    })
}

fn contains_credential_assignment(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "api_key",
        "apikey",
        "access_token",
        "client_secret",
    ]
    .iter()
    .any(|key| {
        [format!("{key}="), format!("{key}:"), format!("{key} =")]
            .iter()
            .filter_map(|needle| lower.find(needle).map(|index| index + needle.len()))
            .any(|start| {
                lower[start..]
                    .trim_start_matches([' ', '\'', '"'])
                    .split(|character: char| {
                        character.is_whitespace() || matches!(character, '\'' | '"' | ',')
                    })
                    .next()
                    .is_some_and(|candidate| candidate.len() >= 8 && candidate != "[redacted]")
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_paths_cover_root_directories_and_credential_files() {
        for path in [
            "secrets/token.txt",
            ".ssh/config",
            ".aws/credentials",
            ".kube/config",
            ".npmrc",
            "infra/terraform.tfstate.backup",
            "certs/service.key",
            "config/service-account.json",
        ] {
            assert!(is_sensitive_path(path), "expected sensitive path: {path}");
        }
        assert!(!is_sensitive_path("src/key.ts"));
    }

    #[test]
    fn common_credential_shapes_are_redacted() {
        for value in [
            "Authorization: Bearer a-long-runtime-token",
            concat!("AWS_ACCESS_KEY_ID=AK", "IA1234567890ABCDEF"),
            "password=correct-horse-battery-staple",
            "postgres://user:password@localhost/db",
            concat!("xo", "xb-123456789-secret"),
        ] {
            assert!(looks_like_secret(value), "expected secret: {value}");
            assert_eq!(redact_secret_text(value).0, REDACTED);
        }
    }
}
