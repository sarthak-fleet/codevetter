// crate_b has no dependency on crate_a. These scoped calls must not resolve to
// same-named functions in crate_a.
pub struct Server;

impl Server {
    pub fn run(&self) {
        let _ = Server::start();
        let _ = Url::parse("http://example.com");
    }

    fn start() -> bool {
        false
    }
}
