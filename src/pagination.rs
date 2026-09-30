use serde::Deserialize;

pub const PAGE_SIZE: usize = 10;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    #[serde(default)]
    pub page: u32,
}

impl PageRequest {
    pub const FETCH_LIMIT: i64 = PAGE_SIZE as i64 + 1;

    pub const fn new(page: u32) -> Self {
        Self { page }
    }

    pub fn offset(self) -> i64 {
        i64::from(self.page) * PAGE_SIZE as i64
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_extra_row_announces_a_next_page() {
        for count in [0, PAGE_SIZE, PAGE_SIZE + 1] {
            let page = Page::from_rows((0..count).collect(), PageRequest::new(3));
            assert_eq!(
                page.items,
                (0..count.min(PAGE_SIZE)).collect::<Vec<_>>(),
                "{count} rows"
            );
            assert_eq!(page.page, 3);
            assert_eq!(
                page.next_page,
                (count > PAGE_SIZE).then_some(4),
                "{count} rows"
            );
        }
    }

    #[test]
    fn the_last_representable_page_never_wraps_around() {
        let request = PageRequest::new(u32::MAX);
        let page = Page::from_rows(vec![(); PAGE_SIZE + 1], request);
        assert_eq!(page.next_page, None);
        assert_eq!(page.items.len(), PAGE_SIZE);
        assert_eq!(request.offset(), i64::from(u32::MAX) * PAGE_SIZE as i64);
    }
}
