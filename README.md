# Echo: Head AimLock

Head AimLock runs while CS2 is the foreground application. It follows the current head hitbox center from the current camera position and stops movement when geometry is missing or focus changes.

The installed standard player rigs use bone 7 (`head_0`); bone 6 is `neck_0`. The `head_0` capsule of the `cstrike` hitbox set runs from local `(-1, 1.8, 0)` to `(3.5, 0.2, 0)`; both endpoints are carried through the bone's rotation and scale, and the midpoint is what the lock aims at. These values were checked against installed `agents/models/` assets on 2026-09-15. Custom rigs need their own mapping. Logs identify this build as `head-capsule-v7` and print both endpoints, so one run shows whether bone 7 lands on a head.

Targets inside the 1.5 degree cone are ranked every pass: 50% crosshair proximity and 50% distance. Lower scores are better. Distance uses a bounded scale with its midpoint at 1000 world units. A new target must improve the score by more than 0.05 to replace a valid current target. Logs include distance, score, and target changes.

Every line carrying an aim point also carries `aim_z`: how far up the target, from its own feet, that point sits. A standing player's eyes are at 64, so `aim_z` answers head-or-body directly — `grep -o 'aim_z=[-0-9.]*' echo.log | sort -n | uniq -c` is the whole check. `bone-probe` prints the same height for both ends of the capsule as `above_origin`, next to that player's own `eye_z`.

Build with `cargo build --release`, then open `target/release/echo.exe`. Close the console window to stop. Windows requests administrator access. Diagnostic output is written to `echo.log` beside the executable.

Mouse calibration and lock settings are constants in `src/aim/mod.rs`. Run `cargo test --lib` to check the geometry and selection rules.
