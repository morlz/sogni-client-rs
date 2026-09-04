use parking_lot::RwLock;
use reqwest::{
    cookie::{CookieStore, Jar},
    header::{HeaderMap, HeaderValue, SET_COOKIE},
};
use url::Url;

pub(crate) struct ClearableCookieStore {
    active: RwLock<CookieState>,
}

struct CookieState {
    generation: u64,
    jar: Jar,
}

impl Default for ClearableCookieStore {
    fn default() -> Self {
        Self {
            active: RwLock::new(CookieState {
                generation: 0,
                jar: Jar::default(),
            }),
        }
    }
}

impl ClearableCookieStore {
    pub(crate) fn clear(&self) {
        let mut state = self.active.write();
        state.generation = state.generation.wrapping_add(1);
        state.jar = Jar::default();
    }

    pub(crate) fn request_header(&self, url: &Url) -> (u64, Option<HeaderValue>) {
        let state = self.active.read();
        (state.generation, state.jar.cookies(url))
    }

    pub(crate) fn store_response(&self, generation: u64, headers: &HeaderMap, url: &Url) {
        let state = self.active.read();
        if state.generation != generation {
            return;
        }
        state
            .jar
            .set_cookies(&mut headers.get_all(SET_COOKIE).iter(), url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_replaces_the_jar_and_rejects_stale_responses() {
        let store = ClearableCookieStore::default();
        let url = Url::parse("https://api.example.test/account").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            SET_COOKIE,
            HeaderValue::from_static("session=secret; Path=/; HttpOnly"),
        );
        let (generation, _) = store.request_header(&url);
        store.store_response(generation, &headers, &url);
        assert_eq!(store.request_header(&url).1.unwrap(), "session=secret");

        store.clear();
        assert!(store.request_header(&url).1.is_none());
        store.store_response(generation, &headers, &url);
        assert!(store.request_header(&url).1.is_none());
    }
}
