//! One-shot: request a free hobbyist Symbolica license key. The key is
//! emailed to the address below. Run with:
//!
//! ```
//! nix-shell -p m4 gmp mpfr libmpc pkg-config \
//!   --run 'cargo run --example symbolica_license'
//! ```

use symbolica::LicenseManager;

fn main() {
    match LicenseManager::request_hobbyist_license("Sol Astrius", "me@danielsol.dev") {
        Ok(()) => println!("ok — check me@danielsol.dev for the license key"),
        Err(e) => {
            eprintln!("error requesting license: {e}");
            std::process::exit(1);
        }
    }
}
