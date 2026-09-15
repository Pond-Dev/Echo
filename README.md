# Echo: Head AimLock

Head AimLock runs while CS2 is the foreground application. It follows the current head bone from the current camera position and stops movement when geometry is missing or focus changes.

Targets inside the 1.5 degree cone are ranked every pass: 50% crosshair proximity and 50% distance. Lower scores are better. Distance uses a bounded scale with its midpoint at 1000 world units. A new target must improve the score by more than 0.05 to replace a valid current target. Logs include distance, score, and target changes.

Build with `cargo build --release`, then open `target/release/echo.exe`. Close the console window to stop. Windows requests administrator access. Diagnostic output is written to `echo.log` beside the executable.

Mouse calibration and lock settings are constants in `src/aim/mod.rs`. Run `cargo test --lib` to check the geometry and selection rules.
