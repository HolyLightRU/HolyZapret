use super::lists::Doc;
use super::paths::Paths;
use std::path::PathBuf;

pub const SERVICES: &[(&str, &[&str])] = &[
    (
        "Steam",
        &[
            "steampowered.com",
            "steamcommunity.com",
            "steamstatic.com",
            "steamcontent.com",
            "steamserver.net",
            "steamgames.com",
            "steam-chat.com",
            "steamusercontent.com",
            "valvesoftware.com",
            "steamcdn-a.akamaihd.net",
            "steamuserimages-a.akamaihd.net",
            "steamstore-a.akamaihd.net",
            "steamcommunity-a.akamaihd.net",
        ],
    ),
    ("Epic Games", &["epicgames.com", "epicgames.dev", "unrealengine.com"]),
    ("Battle.net", &["battle.net", "blizzard.com"]),
    ("Госуслуги", &["gosuslugi.ru"]),
    ("Сбер", &["sberbank.ru", "sber.ru"]),
    ("Т-Банк", &["tbank.ru", "tinkoff.ru"]),
    ("ВТБ", &["vtb.ru"]),
    ("Альфа-Банк", &["alfabank.ru"]),
    ("Яндекс", &["yandex.ru", "ya.ru", "yandex.net"]),
    ("VK", &["vk.com", "vk.ru", "userapi.com", "vkuser.net", "mail.ru"]),
    ("Ozon", &["ozon.ru"]),
    ("Wildberries", &["wildberries.ru", "wb.ru"]),
    ("Авито", &["avito.ru"]),
];

pub fn service_of(domain: &str) -> Option<&'static str> {
    let d = domain.trim().trim_end_matches('.').to_lowercase();
    SERVICES
        .iter()
        .find(|(_, bases)| bases.iter().any(|b| d == *b || d.ends_with(&format!(".{b}"))))
        .map(|(name, _)| *name)
}

pub fn service_domains(domain: &str) -> &'static [&'static str] {
    let d = domain.trim().trim_end_matches('.').to_lowercase();
    SERVICES
        .iter()
        .find(|(_, bases)| bases.iter().any(|b| d == *b || d.ends_with(&format!(".{b}"))))
        .map(|(_, bases)| *bases)
        .unwrap_or(&[])
}

pub fn names(domains: &[String]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for d in domains {
        if let Some(n) = service_of(d) {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub file: PathBuf,
    pub domains: Vec<String>,
}

pub fn in_user_lists(paths: &Paths) -> Vec<Found> {
    super::reset::list_dirs(paths)
        .into_iter()
        .filter_map(|dir| {
            let file = dir.join("list-general-user.txt");
            let doc = Doc::load(&file);
            let domains: Vec<String> = (0..doc.len()).map(|i| doc.entry(i).to_string()).filter(|d| service_of(d).is_some()).collect();
            (!domains.is_empty()).then_some(Found { file, domains })
        })
        .collect()
}

pub fn remove(found: &[Found]) -> anyhow::Result<usize> {
    let mut removed = 0;
    for f in found {
        let mut doc = Doc::load(&f.file);
        let mut i = 0;
        while i < doc.len() {
            if f.domains.iter().any(|d| d.eq_ignore_ascii_case(doc.entry(i))) {
                doc.remove(i);
                removed += 1;
            } else {
                i += 1;
            }
        }
        doc.save(&f.file, super::lists::Kind::Domains)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_are_recognised() {
        assert_eq!(service_of("store.steampowered.com"), Some("Steam"));
        assert_eq!(service_of("gosuslugi.ru"), Some("Госуслуги"));
        assert_eq!(service_of("notsteampowered.com"), None);
        assert_eq!(service_of("youtube.com"), None);
        assert_eq!(names(&["steamcommunity.com".into(), "x.com".into(), "steamstatic.com".into(), "ya.ru".into()]), vec!["Steam", "Яндекс"]);
    }

    #[test]
    fn removes_only_known_domains() {
        let dir = std::env::temp_dir().join(format!("hz-known-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("list-general-user.txt");
        std::fs::write(&file, "x.com\r\nsteampowered.com\r\nrutracker.org\r\nvk.com\r\n").unwrap();
        let found = vec![Found { file: file.clone(), domains: vec!["steampowered.com".into(), "vk.com".into()] }];
        assert_eq!(remove(&found).unwrap(), 2);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x.com\r\nrutracker.org\r\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
