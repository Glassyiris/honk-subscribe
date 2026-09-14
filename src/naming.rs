struct CountryRule {
    flag: &'static str,
    region: &'static str,
    aliases: &'static [&'static str],
}

const COUNTRY_ALIASES: &[CountryRule] = &[
    CountryRule {
        flag: "🇭🇰",
        region: "hk",
        aliases: &["hk", "香港", "hongkong", "hong kong"],
    },
    CountryRule {
        flag: "🇯🇵",
        region: "jp",
        aliases: &["jp", "日本", "japan", "tokyo", "osaka", "🇯🇵"],
    },
    CountryRule {
        flag: "🇸🇬",
        region: "sg",
        aliases: &["sg", "新加坡", "singapore", "sgp"],
    },
    CountryRule {
        flag: "🇺🇸",
        region: "us",
        aliases: &["us", "美国", "united states", "usa"],
    },
    CountryRule {
        flag: "🇰🇷",
        region: "kr",
        aliases: &["kr", "韩国", "korea", "🇰🇷"],
    },
    CountryRule {
        flag: "🇹🇼",
        region: "tw",
        aliases: &["cn", "中国", "china"],
    },
    CountryRule {
        flag: "🇹🇼",
        region: "tw",
        aliases: &["tw", "台湾", "taiwan", "🇹🇼"],
    },
    CountryRule {
        flag: "🇻🇦",
        region: "va",
        aliases: &["vatican", "梵蒂冈"],
    },
    CountryRule {
        flag: "🇬🇧",
        region: "gb",
        aliases: &["uk", "英国", "united kingdom"],
    },
    CountryRule {
        flag: "🇩🇪",
        region: "de",
        aliases: &["de", "德国", "germany"],
    },
    CountryRule {
        flag: "🇫🇷",
        region: "fr",
        aliases: &["fr", "法国", "france"],
    },
    CountryRule {
        flag: "🇦🇺",
        region: "au",
        aliases: &["au", "澳大利亚", "australia"],
    },
    CountryRule {
        flag: "🇳🇱",
        region: "nl",
        aliases: &["nl", "荷兰", "netherlands"],
    },
    CountryRule {
        flag: "🇮🇹",
        region: "it",
        aliases: &["it", "意大利", "italy"],
    },
    CountryRule {
        flag: "🇸🇪",
        region: "se",
        aliases: &["se", "瑞典", "sweden"],
    },
    CountryRule {
        flag: "🇨🇭",
        region: "ch",
        aliases: &["ch", "瑞士", "switzerland"],
    },
    CountryRule {
        flag: "🇷🇺",
        region: "ru",
        aliases: &["ru", "俄罗斯", "russia"],
    },
    CountryRule {
        flag: "🇧🇷",
        region: "br",
        aliases: &["br", "巴西", "brazil"],
    },
    CountryRule {
        flag: "🇮🇳",
        region: "in",
        aliases: &["in", "印度", "india"],
    },
    CountryRule {
        flag: "🇹🇭",
        region: "th",
        aliases: &["th", "泰国", "thailand"],
    },
    CountryRule {
        flag: "🇻🇳",
        region: "vn",
        aliases: &["vn", "越南", "vietnam"],
    },
    CountryRule {
        flag: "🇲🇾",
        region: "my",
        aliases: &["my", "马来西亚", "malaysia"],
    },
    CountryRule {
        flag: "🇵🇭",
        region: "ph",
        aliases: &["ph", "菲律宾", "philippines"],
    },
    CountryRule {
        flag: "🇮🇩",
        region: "id",
        aliases: &["id", "印度尼西亚", "印尼", "indonesia"],
    },
    CountryRule {
        flag: "🇲🇽",
        region: "mx",
        aliases: &["mx", "墨西哥", "mexico"],
    },
    CountryRule {
        flag: "🇦🇷",
        region: "ar",
        aliases: &["ar", "阿根廷", "argentina"],
    },
    CountryRule {
        flag: "🇨🇦",
        region: "ca",
        aliases: &["ca", "加拿大", "canada"],
    },
    CountryRule {
        flag: "🇵🇰",
        region: "pk",
        aliases: &["pk", "巴基斯坦", "pakistan"],
    },
    CountryRule {
        flag: "🇹🇷",
        region: "tr",
        aliases: &["tr", "土耳其", "turkey"],
    },
    CountryRule {
        flag: "🇲🇴",
        region: "mo",
        aliases: &["mo", "澳门", "macao"],
    },
];

const UNKNOWN_REGION: CountryRule = CountryRule {
    flag: "🌐",
    region: "xx",
    aliases: &[],
};

pub fn renamed(original: &str, provider: &str, index: usize) -> String {
    rendered(
        original,
        provider,
        index,
        "{flag} {provider}.{region}.{index}",
    )
}

pub(crate) fn normalize_region(value: &str) -> String {
    if value.trim().eq_ignore_ascii_case("cn") {
        "tw".into()
    } else {
        value.trim().to_ascii_lowercase()
    }
}

pub(crate) fn normalize_proxy_name(value: &str) -> String {
    replace_region_code(&value.replace("🇨🇳", "🇹🇼"), "cn", "tw")
}

pub fn rendered(original: &str, provider: &str, index: usize, template: &str) -> String {
    let original = normalize_proxy_name(original);
    let (flag, region) = find_flag_region(&original)
        .or_else(|| {
            COUNTRY_ALIASES
                .iter()
                .find(|rule| {
                    matches_alias(&original, rule.flag)
                        || rule
                            .aliases
                            .iter()
                            .any(|alias| matches_alias(&original, alias))
                })
                .map(|rule| (rule.flag.to_string(), rule.region.to_string()))
        })
        .unwrap_or_else(|| {
            (
                UNKNOWN_REGION.flag.to_string(),
                UNKNOWN_REGION.region.to_string(),
            )
        });
    let provider = slug(provider);
    template
        .replace("{flag}", &flag)
        .replace("{provider}", &provider)
        .replace("{region}", &region)
        .replace("{index}", &format!("{index:02}"))
        .replace("{name}", original.trim())
        .trim()
        .to_string()
}

pub(crate) fn region(value: &str) -> Option<String> {
    find_flag_region(value)
        .map(|(_, region)| normalize_region(&region))
        .or_else(|| {
            COUNTRY_ALIASES
                .iter()
                .find(|rule| {
                    matches_alias(value, rule.flag)
                        || rule.aliases.iter().any(|alias| matches_alias(value, alias))
                })
                .map(|rule| normalize_region(rule.region))
        })
}

fn find_flag_region(value: &str) -> Option<(String, String)> {
    let mut chars = value.chars().peekable();
    while let Some(first) = chars.next() {
        let Some(&second) = chars.peek() else {
            continue;
        };
        let Some(first_letter) = regional_letter(first) else {
            continue;
        };
        let Some(second_letter) = regional_letter(second) else {
            continue;
        };
        return Some((
            [first, second].into_iter().collect(),
            [first_letter, second_letter].into_iter().collect(),
        ));
    }
    None
}

fn regional_letter(value: char) -> Option<char> {
    let offset = (value as u32).checked_sub(0x1f1e6)?;
    if offset > 25 {
        return None;
    }
    char::from_u32(u32::from(b'a') + offset)
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    for char in value.trim().chars() {
        if char.is_alphanumeric() {
            result.extend(char.to_lowercase());
        } else if !result.is_empty() && !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_end_matches('-').to_string();
    if result.is_empty() {
        "provider".into()
    } else {
        result
    }
}

fn matches_alias(value: &str, alias: &str) -> bool {
    let alias = alias.trim();
    if alias.chars().all(|char| char.is_ascii_alphanumeric()) && alias.len() <= 3 {
        return value
            .split(|char: char| !char.is_alphanumeric())
            .any(|token| {
                token.eq_ignore_ascii_case(alias)
                    || token.strip_prefix(alias).is_some_and(|suffix| {
                        !suffix.is_empty() && suffix.chars().all(|char| char.is_ascii_digit())
                    })
            });
    }
    compact(value).contains(&compact(alias))
}

fn compact(value: &str) -> String {
    value
        .chars()
        .filter(|char| !char.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn replace_region_code(value: &str, from: &str, to: &str) -> String {
    let from = from.as_bytes();
    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut cursor = 0;
    for index in 0..bytes.len().saturating_sub(from.len() - 1) {
        let before = index
            .checked_sub(1)
            .is_some_and(|value| bytes[value].is_ascii_alphanumeric());
        let after = bytes
            .get(index + from.len())
            .is_some_and(u8::is_ascii_alphanumeric);
        if !before
            && !after
            && bytes[index..].len() >= from.len()
            && bytes[index..index + from.len()].eq_ignore_ascii_case(from)
        {
            result.push_str(&value[cursor..index]);
            result.push_str(to);
            cursor = index + from.len();
        }
    }
    result.push_str(&value[cursor..]);
    result
}

#[cfg(test)]
mod tests {
    use super::{COUNTRY_ALIASES, normalize_proxy_name, region, renamed, rendered};

    #[test]
    fn regions_are_inferred_from_names() {
        assert_eq!(region("🇭🇰 yss.hk.01"), Some("hk".into()));
        assert_eq!(region("香港 01"), Some("hk".into()));
        assert_eq!(region("edge-node"), None);
    }

    #[test]
    fn names_use_provider_region_and_index() {
        assert_eq!(renamed("香港 01", "YSS", 1), "🇭🇰 yss.hk.01");
        assert_eq!(
            renamed("Tokyo JP-01", "Main Provider", 2),
            "🇯🇵 main-provider.jp.02"
        );
        assert_eq!(renamed("🇲🇳 蒙古 01", "YSS", 1), "🇲🇳 yss.mn.01");
        assert_eq!(renamed("edge-node", "Main", 2), "🌐 main.xx.02");
    }

    #[test]
    fn templates_support_the_same_parts() {
        assert_eq!(
            rendered("香港 01", "YSS", 1, "{flag} {provider}.{region}.{index}"),
            "🇭🇰 yss.hk.01"
        );
        assert_eq!(
            rendered("香港 01", "YSS", 1, "{index} · {name}"),
            "01 · 香港 01"
        );
    }

    #[test]
    fn proxy_cn_is_normalized_to_taiwan() {
        assert_eq!(region("nexi.cn.01"), Some("tw".into()));
        assert_eq!(
            rendered(
                "nexi.cn.01",
                "nexi",
                1,
                "{flag} {provider}.{region}.{index}"
            ),
            "🇹🇼 nexi.tw.01"
        );
        assert_eq!(normalize_proxy_name("🇨🇳 nexi.cn.01"), "🇹🇼 nexi.tw.01");
    }

    #[test]
    fn generated_names_are_repeatable() {
        for rule in COUNTRY_ALIASES {
            let first = renamed(rule.aliases[0], "Main", 1);
            assert!(first.starts_with(rule.flag));
            assert_eq!(first, renamed(&first, "Main", 1));
        }
    }
}
