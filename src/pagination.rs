use serde::Deserialize;

pub const PAGE_SIZE: usize = 10;
pub const ROSTER_PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PageRequest {
    pub page: u32,
    #[serde(skip)]
    size: usize,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self::new(0)
    }
}

impl PageRequest {
    pub const fn new(page: u32) -> Self {
        Self::sized(page, PAGE_SIZE)
    }

    pub const fn sized(page: u32, size: usize) -> Self {
        Self { page, size }
    }

    pub fn offset(self) -> i64 {
        i64::from(self.page) * self.size as i64
    }

    pub fn fetch_limit(self) -> i64 {
        self.size as i64 + 1
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
        let next_page = (items.len() > request.size)
            .then(|| request.page.checked_add(1))
            .flatten();
        items.truncate(request.size);
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
        for size in [PAGE_SIZE, ROSTER_PAGE_SIZE] {
            let request = PageRequest::sized(3, size);
            assert_eq!(request.fetch_limit(), size as i64 + 1);
            for count in [0, size, size + 1] {
                let page = Page::from_rows((0..count).collect(), request);
                assert_eq!(
                    page.items,
                    (0..count.min(size)).collect::<Vec<_>>(),
                    "{count} of {size} rows"
                );
                assert_eq!(page.page, 3);
                assert_eq!(
                    page.next_page,
                    (count > size).then_some(4),
                    "{count} of {size} rows"
                );
            }
        }
    }

    #[test]
    fn a_page_read_from_a_query_has_the_default_size() {
        let request: PageRequest =
            serde_json::from_value(serde_json::json!({ "page": 3 })).unwrap();

        assert_eq!(request, PageRequest::new(3));
        assert_eq!(
            serde_json::from_value::<PageRequest>(serde_json::json!({})).unwrap(),
            PageRequest::default()
        );
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
