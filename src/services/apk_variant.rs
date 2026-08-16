/// Detects the CPU architecture / APK variant label from a given filename.
/// Returns standard canonical labels: "universal", "arm64-v8a", "armeabi-v7a", "x86_64", "x86"
pub fn detect_variant(filename: &str) -> Option<&'static str> {
    let f = filename.to_lowercase();
    if f.contains("universal") {
        return Some("universal");
    }

    if f.contains("arm64-v8a")
        || f.contains("arm64_v8a")
        || f.contains("arm64")
        || f.contains("aarch64")
    {
        return Some("arm64-v8a");
    }

    if f.contains("armeabi-v7a")
        || f.contains("armeabi_v7a")
        || f.contains("armv7a")
        || f.contains("armv7")
        || f.contains("armeabi")
    {
        return Some("armeabi-v7a");
    }

    if f.contains("x86_64") || f.contains("x86-64") || f.contains("amd64") {
        return Some("x86_64");
    }

    if f.contains("x86") || f.contains("i686") || f.contains("i386") {
        return Some("x86");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_variant() {
        assert_eq!(detect_variant("app-universal.apk"), Some("universal"));
        assert_eq!(
            detect_variant("app-release-arm64-v8a.apk"),
            Some("arm64-v8a")
        );
        assert_eq!(detect_variant("App_v2.1_arm64.apk"), Some("arm64-v8a"));
        assert_eq!(
            detect_variant("app-armeabi-v7a-signed.apk"),
            Some("armeabi-v7a")
        );
        assert_eq!(detect_variant("app-x86_64.apk"), Some("x86_64"));
        assert_eq!(detect_variant("app-x86.apk"), Some("x86"));
        assert_eq!(detect_variant("my-application.apk"), None);
    }
}
