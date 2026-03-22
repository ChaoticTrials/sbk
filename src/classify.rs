use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Group {
    Mca = 0,
    Nbt = 1,
    Json = 2,
    Raw = 3,
}

impl Group {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Mca),
            1 => Some(Self::Nbt),
            2 => Some(Self::Json),
            3 => Some(Self::Raw),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

pub fn classify(path: &Path) -> Group {
    match path.extension().and_then(|e| e.to_str()) {
        Some("mca") => Group::Mca,
        Some("dat") | Some("dat_old") => Group::Nbt,
        Some("json") => Group::Json,
        _ => Group::Raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_mca() {
        assert_eq!(classify(Path::new("region/r.0.0.mca")), Group::Mca);
    }

    #[test]
    fn classify_nbt() {
        assert_eq!(classify(Path::new("level.dat")), Group::Nbt);
        assert_eq!(classify(Path::new("level.dat_old")), Group::Nbt);
    }

    #[test]
    fn classify_json() {
        assert_eq!(classify(Path::new("advancements/player.json")), Group::Json);
    }

    #[test]
    fn classify_raw() {
        assert_eq!(classify(Path::new("icon.png")), Group::Raw);
        assert_eq!(classify(Path::new("session.lock")), Group::Raw);
    }

    #[test]
    fn group_roundtrip() {
        for v in 0u8..4 {
            let g = Group::from_u8(v).unwrap();
            assert_eq!(g.as_u8(), v);
        }
        assert!(Group::from_u8(4).is_none());
    }
}
