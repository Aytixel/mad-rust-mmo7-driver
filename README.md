# Mad Rust MMO7 Driver

This is the Mad Rust driver for the MMO7 mouse.

**⚠️ Please take attention to this, Mad Rust is not an official software and potentially unstable, I am not responsible for any damage at your equipment.**

For now, this software only supports MMO7 mouse, but feel free to create your own driver.
Moreover, it is not a real driver, which allows it to be cross-platform.

## Links
- [Mad Rust](https://github.com/Aytixel/mad-rust), the software to modify devices configuration.
- [Mad Rust MMO7 Driver](https://github.com/Aytixel/mad-rust-mmo7-driver), a compatible driver for the Mad Catz MMO7 mouse.
- [Mad Rust Util](https://github.com/Aytixel/mad-rust-util), a set of utility function, to help build driver or software compatible with Mad Rust.

# Windows installation

On Windows, you will have to change the driver of your equipment, to Winusb or libusb.
For that I recommend you to use the [Zadig](https://zadig.akeo.ie) software to change it.
**Else Mad Rust MMO7 Driver will not be able to access it.**

# Linux installation

On Linux if you don't have libxdo installed, you will need to install it with : **"sudo apt-get install libxdo-dev".**

# Running the driver

On each system, it's recommended to run the driver in admin mode.
Otherwise, it might cause some problems due to permissions.

# Building installer

## Debian

You can build debian package by installing [cargo-deb](https://crates.io/crates/cargo-deb), and running : **"cargo deb".**
It will create a new debian folder in the target folder, containing the .deb file created.