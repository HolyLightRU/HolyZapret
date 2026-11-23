use std::fs;
use std::path::Path;

const CP866: [char; 128] = ['\u{410}', '\u{411}', '\u{412}', '\u{413}', '\u{414}', '\u{415}', '\u{416}', '\u{417}', '\u{418}', '\u{419}', '\u{41a}', '\u{41b}', '\u{41c}', '\u{41d}', '\u{41e}', '\u{41f}', '\u{420}', '\u{421}', '\u{422}', '\u{423}', '\u{424}', '\u{425}', '\u{426}', '\u{427}', '\u{428}', '\u{429}', '\u{42a}', '\u{42b}', '\u{42c}', '\u{42d}', '\u{42e}', '\u{42f}', '\u{430}', '\u{431}', '\u{432}', '\u{433}', '\u{434}', '\u{435}', '\u{436}', '\u{437}', '\u{438}', '\u{439}', '\u{43a}', '\u{43b}', '\u{43c}', '\u{43d}', '\u{43e}', '\u{43f}', '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}', '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}', '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}', '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}', '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}', '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}', '\u{440}', '\u{441}', '\u{442}', '\u{443}', '\u{444}', '\u{445}', '\u{446}', '\u{447}', '\u{448}', '\u{449}', '\u{44a}', '\u{44b}', '\u{44c}', '\u{44d}', '\u{44e}', '\u{44f}', '\u{401}', '\u{451}', '\u{404}', '\u{454}', '\u{407}', '\u{457}', '\u{40e}', '\u{45e}', '\u{b0}', '\u{2219}', '\u{b7}', '\u{221a}', '\u{2116}', '\u{a4}', '\u{25a0}', '\u{a0}'];
const CP437: [char; 128] = ['\u{c7}', '\u{fc}', '\u{e9}', '\u{e2}', '\u{e4}', '\u{e0}', '\u{e5}', '\u{e7}', '\u{ea}', '\u{eb}', '\u{e8}', '\u{ef}', '\u{ee}', '\u{ec}', '\u{c4}', '\u{c5}', '\u{c9}', '\u{e6}', '\u{c6}', '\u{f4}', '\u{f6}', '\u{f2}', '\u{fb}', '\u{f9}', '\u{ff}', '\u{d6}', '\u{dc}', '\u{a2}', '\u{a3}', '\u{a5}', '\u{20a7}', '\u{192}', '\u{e1}', '\u{ed}', '\u{f3}', '\u{fa}', '\u{f1}', '\u{d1}', '\u{aa}', '\u{ba}', '\u{bf}', '\u{2310}', '\u{ac}', '\u{bd}', '\u{bc}', '\u{a1}', '\u{ab}', '\u{bb}', '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}', '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}', '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}', '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}', '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}', '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}', '\u{3b1}', '\u{df}', '\u{393}', '\u{3c0}', '\u{3a3}', '\u{3c3}', '\u{b5}', '\u{3c4}', '\u{3a6}', '\u{398}', '\u{3a9}', '\u{3b4}', '\u{221e}', '\u{3c6}', '\u{3b5}', '\u{2229}', '\u{2261}', '\u{b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{f7}', '\u{2248}', '\u{b0}', '\u{2219}', '\u{b7}', '\u{221a}', '\u{207f}', '\u{b2}', '\u{25a0}', '\u{a0}'];
const CP1251: [char; 128] = ['\u{402}', '\u{403}', '\u{201a}', '\u{453}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{20ac}', '\u{2030}', '\u{409}', '\u{2039}', '\u{40a}', '\u{40c}', '\u{40b}', '\u{40f}', '\u{452}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{98}', '\u{2122}', '\u{459}', '\u{203a}', '\u{45a}', '\u{45c}', '\u{45b}', '\u{45f}', '\u{a0}', '\u{40e}', '\u{45e}', '\u{408}', '\u{a4}', '\u{490}', '\u{a6}', '\u{a7}', '\u{401}', '\u{a9}', '\u{404}', '\u{ab}', '\u{ac}', '\u{ad}', '\u{ae}', '\u{407}', '\u{b0}', '\u{b1}', '\u{406}', '\u{456}', '\u{491}', '\u{b5}', '\u{b6}', '\u{b7}', '\u{451}', '\u{2116}', '\u{454}', '\u{bb}', '\u{458}', '\u{405}', '\u{455}', '\u{457}', '\u{410}', '\u{411}', '\u{412}', '\u{413}', '\u{414}', '\u{415}', '\u{416}', '\u{417}', '\u{418}', '\u{419}', '\u{41a}', '\u{41b}', '\u{41c}', '\u{41d}', '\u{41e}', '\u{41f}', '\u{420}', '\u{421}', '\u{422}', '\u{423}', '\u{424}', '\u{425}', '\u{426}', '\u{427}', '\u{428}', '\u{429}', '\u{42a}', '\u{42b}', '\u{42c}', '\u{42d}', '\u{42e}', '\u{42f}', '\u{430}', '\u{431}', '\u{432}', '\u{433}', '\u{434}', '\u{435}', '\u{436}', '\u{437}', '\u{438}', '\u{439}', '\u{43a}', '\u{43b}', '\u{43c}', '\u{43d}', '\u{43e}', '\u{43f}', '\u{440}', '\u{441}', '\u{442}', '\u{443}', '\u{444}', '\u{445}', '\u{446}', '\u{447}', '\u{448}', '\u{449}', '\u{44a}', '\u{44b}', '\u{44c}', '\u{44d}', '\u{44e}', '\u{44f}'];

const PAGES: [&[char; 128]; 3] = [&CP866, &CP437, &CP1251];
const DIRS: [&str; 7] = ["bin", "lists", "exp-list", "utils", "additional", "experimental", super::flowseal::DIR];

fn encode(s: &str, page: &[char; 128]) -> Option<Vec<u8>> {
    s.chars()
        .map(|c| if c.is_ascii() { Some(c as u8) } else { page.iter().position(|&p| p == c).map(|i| (i + 128) as u8) })
        .collect()
}

fn cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c))
}

fn step(name: &str) -> Option<String> {
    if name.is_ascii() {
        return None;
    }
    PAGES.iter().find_map(|page| String::from_utf8(encode(name, page)?).ok().filter(|f| f != name))
}

fn boxes(s: &str) -> bool {
    s.chars().any(|c| ('\u{2500}'..='\u{259f}').contains(&c))
}

pub fn repair(name: &str) -> Option<String> {
    let mut cur = name.to_string();
    for _ in 0..3 {
        match step(&cur) {
            Some(next) => cur = next,
            None => break,
        }
    }
    (cur != name && cyrillic(&cur) && !boxes(&cur)).then_some(cur)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::read(a), fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn repair_dir(dir: &Path, only_bat: bool, out: &mut Vec<String>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if only_bat && !name.to_lowercase().ends_with(".bat") {
            continue;
        }
        let Some(fixed) = repair(&name) else { continue };
        let to = dir.join(&fixed);
        if to.exists() {
            let gone = if same_file(&e.path(), &to) { fs::remove_file(e.path()).is_ok() } else { super::relocate::recycle(&e.path()) };
            if gone {
                out.push(fixed);
            }
            continue;
        }
        if fs::rename(e.path(), &to).is_ok() {
            out.push(fixed);
        }
    }
}

pub fn oem(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| if b < 128 { b as char } else { CP866[(b - 128) as usize] }).collect(),
    }
}

pub fn repair_tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    repair_dir(root, true, &mut out);
    for d in DIRS {
        repair_dir(&root.join(d), false, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broken(s: &str, page: &[char; 128]) -> String {
        s.bytes().map(|b| if b < 128 { b as char } else { page[(b - 128) as usize] }).collect()
    }

    #[test]
    fn mojibake_names_repaired() {
        let real = "R01 general, подмена ts и badseq.bat";
        for page in PAGES {
            let bad = broken(real, page);
            assert_ne!(bad, real);
            assert_eq!(repair(&bad).as_deref(), Some(real));
        }
        let twice = broken(&broken(real, &CP866), &CP866);
        assert_eq!(repair(&twice).as_deref(), Some(real));
        let id = format!("experimental\\{}", broken(real, &CP866).trim_end_matches(".bat"));
        assert_eq!(repair(&id), Some(format!("experimental\\{}", real.trim_end_matches(".bat"))));
        for fine in ["general.bat", "R06 general, наложение у SNI.bat", "Ёлка.bat", "ВЁ.txt", "Мои домены"] {
            assert_eq!(repair(fine), None, "{fine}");
        }
    }

    #[test]
    fn oem_output_decoded() {
        assert_eq!(oem("Ок".as_bytes()), "Ок");
        assert_eq!(oem(&[0x8e, 0xaa, 0x2e]), "Ок.");
    }

    #[test]
    fn tree_repaired_in_place() {
        let root = std::env::temp_dir().join(format!("hz-names-{}", std::process::id()));
        let exp = root.join("experimental");
        fs::create_dir_all(&exp).unwrap();
        let good = "R02 general, подмена badseq.bat";
        fs::write(exp.join(broken(good, &CP866)), "a").unwrap();
        let twin = "R03 general, подмена md5sig.bat";
        fs::write(exp.join(twin), "b").unwrap();
        fs::write(exp.join(broken(twin, &CP866)), "b").unwrap();
        fs::write(root.join(broken("Фото.jpg", &CP866)), "p").unwrap();
        let fixed = repair_tree(&root);
        assert_eq!(fixed.len(), 2);
        assert!(exp.join(good).is_file() && exp.join(twin).is_file());
        assert_eq!(fs::read_dir(&exp).unwrap().count(), 2);
        assert!(root.join(broken("Фото.jpg", &CP866)).is_file());
        fs::remove_dir_all(&root).unwrap();
    }
}

