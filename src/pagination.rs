use serde::Deserialize;

pub const PAGE_SIZE: usize = 10;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    #[serde(default)]
    pub page: u32,
}

impl PageRequest {
    pub const fn new(page: u32) -> Self {
        Self { page }
    }

    pub fn offset(self) -> i64 {
        i64::from(self.page) * PAGE_SIZE as i64
    }

    pub fn fetch_limit(self) -> i64 {
        PAGE_SIZE as i64 + 1
    }
}

#[derive(Clone, Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub next_page: Option<u32>,
}

impl<T> Page<T> {
    pub fn from_rows(mut items: Vec<T>, request: PageRequest) -> Self {
        let next_page = (items.len() > PAGE_SIZE)
            .then(|| request.page.checked_add(1))
            .flatten();
        items.truncate(PAGE_SIZE);
        Self {
            items,
            page: request.page,
            next_page,
        }
    }
}
