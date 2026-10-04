// Общие сетевые примитивы: один HTTP-клиент и один tokio-рантайм на всё приложение.
use std::sync::OnceLock;

// Один клиент на всё приложение — переиспользует TLS-соединения
pub(crate) fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .gzip(true)
            .build()
            .expect("reqwest client")
    })
}

// Один рантайм на всё приложение
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}
