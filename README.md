# Echo: Head AimLock

Head AimLock runs while CS2 is the foreground application. It follows the current head hitbox center from the current camera position and stops movement when geometry is missing or focus changes.

AimLock only runs while holding a known gun or Zeus. Switching to a knife, grenade or C4 clears the target and pending movement; switching back resumes targeting. Missing or unreadable weapon data also pauses aim (`reason=weapon-disabled` in the log). Item definition IDs follow the [CounterStrikeSharp item definitions](https://docs.cssharp.dev/api/CounterStrikeSharp.API.Modules.Entities.Constants.ItemDefinition.html).

The installed standard player rigs use bone 7 (`head_0`); bone 6 is `neck_0`. The `head_0` capsule of the `cstrike` hitbox set runs from local `(-1, 1.8, 0)` to `(3.5, 0.2, 0)`; both endpoints are carried through the bone's rotation and scale, and the midpoint is what the lock aims at. These values were checked against installed `agents/models/` assets on 2026-09-15. Custom rigs need their own mapping. Logs identify this build as `head-ease-v8` and print both endpoints, so one run shows whether bone 7 lands on a head.

Targets inside the 10 degree cone are ranked every pass: 50% crosshair proximity and 50% distance. Lower scores are better. Distance uses a bounded scale with its midpoint at 1000 world units. A new target must improve the score by more than 0.05 to replace a valid current target. Logs include distance, score, and target changes.

Mouse movement eases toward the current head with a 35 ms response time constant and a soft speed limit of 2500 counts per second. Smaller `RESPONSE_MS` values pull harder; larger values follow more gently. Fractional counts accumulate so small corrections still reach the 0.03 degree deadzone. Target changes, target loss, and blocked input clear that remainder. Long loop delays contribute at most 16 ms of movement; acquisition uses at most 8 ms.

Every line carrying an aim point also carries `aim_z`: how far up the target, from its own feet, that point sits. A standing player's eyes are at 64, so `aim_z` answers head-or-body directly — `grep -o 'aim_z=[-0-9.]*' echo.log | sort -n | uniq -c` is the whole check. `bone-probe` prints the same height for both ends of the capsule as `above_origin`, next to that player's own `eye_z`.

Build with `cargo build --release`, then open `target/release/echo.exe`. Close the console window to stop. Windows requests administrator access. Diagnostic output is written to `echo.log` beside the executable.

Open CS2 before Echo. The embedded dumper from `cs2-killtimer` scans the running game once at startup and resolves module addresses, schema fields, entity layout and the bone-array pointer. No offset download or rebuild is needed when addresses move. Missing required fields, outdated scan patterns or an unsupported bone-transform layout stop startup with an error; there is no fallback to stale dumps. The log records `source=embedded-dumper` and the resolved values. The vendored dumper retains its MIT license in `vendor/cs2_dumper/LICENSE`.

For a read-only live check, run `cargo test --lib live_offsets -- --ignored --nocapture` from an administrator terminal with CS2 open. This attaches and reads game state without sending mouse input.

Mouse calibration and lock settings are constants in `src/aim/mod.rs`. Run `cargo test --lib` to check the geometry and selection rules.
