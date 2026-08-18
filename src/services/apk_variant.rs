/// Detects the CPU architecture / package variant label from a given filename.
/// Returns standard canonical labels: "universal", "arm64-v8a", "armeabi-v7a", "x86_64", "x86", "archive"
pub fn detect_variant(filename: &str) -> Option<&'static str> {
    let f = filename.to_lowercase();
    if f.contains("universal") {
        return Some("universal");
    }

    if f.contains("arm64-v8a")
        || f.contains("arm64_v8a")
        || f.contains("arm64")
        || f.contains("aarch64")
        || f.contains("armv8")
        || f.contains("v8a")
    {
        return Some("arm64-v8a");
    }

    if f.contains("armeabi-v7a")
        || f.contains("armeabi_v7a")
        || f.contains("armv7a")
        || f.contains("armv7")
        || f.contains("armeabi")
        || f.contains("v7a")
    {
        return Some("armeabi-v7a");
    }

    if f.contains("x86_64") || f.contains("x86-64") || f.contains("amd64") {
        return Some("x86_64");
    }

    if f.contains("x86") || f.contains("i686") || f.contains("i386") {
        return Some("x86");
    }

    if f.ends_with(".zip")
        || f.ends_with(".7z")
        || f.ends_with(".tar.gz")
        || f.ends_with(".tar.xz")
        || f.ends_with(".tgz")
        || f.ends_with(".rar")
    {
        return Some("archive");
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
        assert_eq!(detect_variant("tools.7z"), Some("archive"));
        assert_eq!(detect_variant("module.zip"), Some("archive"));
        assert_eq!(detect_variant("package.tar.gz"), Some("archive"));
        assert_eq!(detect_variant("module-arm64.zip"), Some("arm64-v8a"));
        assert_eq!(detect_variant("my-application.apk"), None);
    }
}
