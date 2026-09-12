# บัญชีการปฏิเสธ (fail-closed harvest)

อ่านจาก `worker/tick/` ของโปรดักต์เดิม — 5,185 บรรทัด 9 ไฟล์
เป้าหมาย: แจกแจงทุก early return / guard / การปฏิเสธ พร้อมเหตุผลที่โค้ดให้ไว้
สถานะ: **กำลังทำ**

---

## คลังคำของการปฏิเสธที่มีอยู่แล้ว (นับได้ใน session จริง)

| คลัง | จำนวน | ค่า |
|---|---|---|
| `AimBlockReason` | 20 | None · ProcessAttach · ClientModule · GameResources · RawInput · ModelCatalog · WeaponCatalog · MapVisibility · Settings · ControllerConfiguration · ActivationReleaseRequired · ControllerClock · LocalIdentity · FlashEvidence · UnsupportedWeapon · TargetFrame · PlayerMotion · TargetProof · FinalAuthorization · SendInput |
| `SelectionReleaseCause` | 11 | SessionEnded · ReadinessBlocked · ActivationLost · LocalSafetyBlock · LifecycleReleased · RetainedTargetInvalid · NoCandidate · FinalAuthorizationRejected · ProposalAbandoned · GateRejected · RecoilLaneTick |
| `StagedCaptureCause` | 21 | ReadUnavailable · IdentityChanged · CoverageIncomplete · OccupancyChanged · CatalogChanged · LeaseInvalid · ModelSetUnavailable · TransformSpanUnavailable · RootTransformInvalid · CoreGeometryInvalid · EvidenceOrderInvalid · OverBudget · TargetTeamUnavailable · TargetPawnUnavailable · TargetHealthAndLifeStateUnavailable · TargetHealthUnavailable · TargetLifeStateUnavailable · TargetHealthOutOfRange · TargetZeroHealthAliveState · TargetPositiveHealthDeadState · CoreCapsuleTransformInvalid |
| `SprayHardResetCause` | 4 | WeaponChanged · ShotRollback · BurstEnd · SprayToFirstShot |

รวม **56 สาเหตุที่มีชื่อ** — และนั่นคือแค่คลังคำ ไม่ใช่จำนวนจุดที่ปฏิเสธ

---

## ⚠ สิ่งที่หักล้างโครงที่เสนอไว้

### 1. การปฏิเสธไม่ใช่ `Err(เหตุผล)` — มันคือ `Err(เหตุผล) + สูตรการรื้อเฉพาะตัว`

ทุกจุดที่ปฏิเสธใน `select_proposal` **รื้อสถานะคนละชุดกัน** ตัวอย่าง:

| ปฏิเสธเพราะ | เคลียร์ echo | เคลียร์ motion window | require release | จบ session |
|---|---|---|---|---|
| frame ใช้ไม่ได้ | ✅ | — | — | — |
| binding หาย | — | — | ✅ | — |
| หลังยิงไม่มีเจ้าของ | ✅ | — | — | — |
| จัดอันดับไม่ได้ใคร (IdleNoAcquisition) | เก็บ | **เก็บไว้** ← เจตนา | — | — |
| control proof หาย | ✅ | เก็บถ้า pending | — | — |
| ticket ทรัพยากรหมดอายุ | — | — | ✅ | ✅ |
| ไม่มีตำแหน่งตา | ✅ | เก็บถ้า pending | — | — |

**นี่คือสาเหตุจริงที่ function ยาว** — ไม่ใช่ "guard เยอะ" แต่คือ **guard แต่ละตัวมีสูตรรื้อของตัวเอง**

`exits.rs` มีอยู่เพราะสูตรบางตัวใช้ร่วมกัน — แต่มีแค่ **3 สูตร** ที่แชร์ได้ ที่เหลือเขียนสดในที่และไม่ซ้ำกันเลย

### 2. แต่การมี seam แก้ปัญหานี้ทิ้งไปเลย

สูตรการรื้อมีอยู่**เพราะโค้ดปัจจุบันกลายพันธุ์สถานะร่วมระหว่างทาง** (`state.control_session.*`)

ใน fold ที่บริสุทธิ์ ไม่มีอะไรให้รื้อ — **"รื้อ" กลายเป็น "คืน successor ที่ไม่มีฟิลด์นั้น"**

```
เดิม:  ปฏิเสธ → เคลียร์ echo, เคลียร์ context, ปล่อย lock, เคลียร์ window  (4 คำสั่ง)
ใหม่:  ปฏิเสธ → คืน successor ที่ไม่มีของพวกนั้นตั้งแต่แรก                (1 ค่า)
```

**โค้ดการรื้อทั้งหมดคือความซับซ้อนที่เกิดจากการกลายพันธุ์ ไม่ใช่จากปัญหา** — และนี่คือหลักฐานที่หนักที่สุดที่เจอว่า seam คุ้ม

**แต่** ต้องแลกมาด้วย: type ของ successor ต้องแสดงความหลากหลายทั้งหมดนี้ได้ นั่นคืองานออกแบบจริงที่ยังไม่ได้ทำ

### 3. การปฏิเสธมีมากกว่าหนึ่ง "ชนิดของทางออก"

- `TickExit::Sleep(Fast)` — จบ tick นี้ tick หน้าลองใหม่
- `TickExit::EndSession(DormantRequireRelease)` — จบ **session** ต้องปล่อยปุ่มก่อนถึงจะเริ่มใหม่ได้

การปฏิเสธจึงต้องพก**ระดับความรุนแรง** ไม่ใช่แค่เหตุผล

### 4. มีเครื่องสถานะวงจรชีวิตที่ยังไม่มีใครพูดถึงเลย

`LifecycleTransition`: Idle · Locked · **EnteredEvidenceGap** · **EvidenceGap** · **EvidenceGapExpired** · Released · Recovered · **EnteredKillCooldown** · **KillCooldown** · CooldownFinished

- **Evidence Gap** — เจ้าของหายจากหลักฐานชั่วคราว มี **deadline เดียว 30 ms** ที่ lifecycle เป็นเจ้าของ ระหว่างนั้น Echo ถือ key ไว้และ `controller_context` ถูกย้ายไปเป็น `gap_context`
- **Kill Cooldown** — ช่วงหลังเป้าตาย
- ทั้งสองอย่างนี้**ไม่มีใน glossary และไม่มีใน INVARIANTS** แต่เป็นกลไกกลางของการผูกเป้า

---

## 🔴 พบกฎที่ตายแล้วชิ้นที่ 4 — และมันทำให้ต้องแก้ spec

ใน `select_proposal` มีคอมเมนต์ว่า:

> *"Post-shot phases may retain one already-authorized owner only as a safety owner; **they never target-steer**, acquire, or retarget."*

**"never target-steer" คือกฎ v63 ที่ A1 ฆ่าไปแล้ว** แต่โค้ดใต้คอมเมนต์ไม่ได้ทำตามคอมเมนต์ — มันออกจาก tick เฉพาะเมื่อ **ไม่มีเจ้าของ** ถ้ามีเจ้าของก็เดินต่อไปนำเป้าตามปกติ

### สิ่งที่ต้องแก้ใน spec

เปิดนิยาม predicate จริงแล้ว:

```
allows_target_acquisition()  →  true  ทุก phase
allows_target_steering()     →  true  ทุก phase
allows_safe_entry_hold()     →  PreShotAim เท่านั้น      ← gate จริง
delivers_spray_recoil()      →  SprayRecoilOnly เท่านั้น  ← gate จริง
```

พร้อม test ที่ assert ทั้งสองข้อ เหตุผลในโค้ด:
> *"การนำเป้าในทุก phase ไร้ประโยชน์ถ้าไม่จับเป้าในทุก phase"*

และเหตุผลที่ยังเก็บเป็น predicate ที่มีชื่อแทนการเขียน `true` ทิ้งไว้:
> *"เก็บเป็น predicate ที่มีชื่อ ไม่ inline เป็น true เพื่อให้นโยบายเฉพาะ phase ในอนาคตมีที่เดียวที่จะเอา gate กลับมา"*

**สรุป: CONTEXT.md ประโยค *"no post-shot phase may steer, acquire, or retarget"* ตายทั้งประโยค ไม่ใช่ครึ่งเดียว**

สิ่งที่ยัง gate ตามจำนวนนัดจริงมีแค่สองอย่าง: **Safe Entry Hold** (ก่อนยิงเท่านั้น) และ **การส่ง spray recoil** (ยืนยันสเปรย์แล้วเท่านั้น)

ส่วน A5 *"ไม่เปลี่ยนเป้าอัตโนมัติกลางการกดค้าง"* **ยังจริง** แต่ถูกบังคับด้วยการคงเจ้าของไว้ ไม่ใช่ด้วย gate ของ phase

### 🔴 โค้ดตายชิ้นที่ 2 — และเจอด้วยเหตุผลเดียวกับชิ้นแรก

ใน `select_proposal`:

```
if !control_phase.allows_target_acquisition() && committed_binding.is_none() {
```

`allows_target_acquisition()` เป็น `const fn` ที่คืน `true` เสมอ → `!true` = false → **ทั้งกิ่งนี้ไม่เคยทำงาน**

กิ่งนั้นมี:
- การบันทึก `post_shot_no_owner` พร้อมฟิลด์วินิจฉัย 9 ตัว
- `record_post_shot_no_owner_edges`
- การเคลียร์สถานะทั้งชุด

**ทั้งหมดเป็นโค้ดตาย** และคอมเมนต์ที่นำหน้ามันยังอ้างกฎ v63 อยู่

นี่คือ**รูปแบบเดียวกับ spray delivery lane** ที่ตายมา 15 เวอร์ชัน: กฎเปลี่ยน → predicate ถูกทำให้เป็น `true`/`false` คงที่ → กิ่งที่อยู่ใต้มันตาย → **ไม่มีอะไรบอก** เพราะ compiler มองว่าเป็นโค้ดที่เข้าถึงได้

**บทเรียนสำหรับ Echo:** predicate ที่ถูกลดรูปเป็นค่าคงที่คือสัญญาณว่ามีกิ่งตายอยู่ใต้มัน ต้องมีวิธีจับ — ตัวนับที่ไม่เคยขยับใน capture จริงคือดัชนีที่ถูกที่สุด

---

## 1. `select_proposal` — phase 5 (selection)

ไฟล์ 546 บรรทัด · function ~492 บรรทัด

### จุดที่ปฏิเสธ (7 จุด · 10 เส้นทาง)

**S1 — staged frame ใช้เป็นผู้สมัครไม่ได้**
ปล่อย lock · เคลียร์เป้าใน Echo · `NoCandidate`
บันทึก `frame_incomplete` พร้อม: stage · cause · scope · stage_flags · cause_flags · issue_overflow · จำนวน observation · จำนวน observation ที่ invalid · มี identity ที่ไม่รู้จักมั้ย
*(จำกัดไว้แค่ tick แรกของช่วงที่ถูกบล็อก และทุกครั้งที่สาเหตุหลักเปลี่ยน — สังเกตอย่างเดียว)*
block `TargetFrame` · reset aim lock · เคลียร์ correction_reference / controller_context / gap_context → **Sleep(Fast)**

**S2 — binding ที่ commit ไว้ตรวจไม่ผ่าน**
`exit_committed_binding_lost`: require_release · release_for_external_invalidation · `RetainedTargetInvalid` · block `TargetProof` → **Sleep(Fast)**

**S3 — phase ไม่อนุญาตให้จับเป้า และไม่มีเจ้าของเดิม**
> เหตุผลในโค้ด: phase หลังยิงเก็บเจ้าของที่อนุญาตแล้วไว้ได้ในฐานะเจ้าของเพื่อความปลอดภัยเท่านั้น ถ้าไม่มีเจ้าของนั้น ก็ไม่มีอำนาจสำหรับนโยบายนัดธรรมชาติหรือ spray

บันทึกขอบของนัดที่เกิดตอนไม่มีเจ้าของ (`post_shot_no_owner`) พร้อม: press_sequence · burst_sequence · shot_advance_count · burst_start_reason · burst_previous_shots · burst_start_shots · อายุตั้งแต่เห็นนัดครั้งแรก/ครั้งสุดท้าย · shots_fired
เคลียร์ทุกอย่าง · `Idle` → **Sleep(Fast)**

**S4 — จัดอันดับครบแล้วแต่ไม่มีใครผ่าน** (4 เส้นทางย่อย)
> เหตุผลในโค้ด: การจัดอันดับที่ครบถ้วนแล้วไม่อนุญาตใครเลย **ใช้ basis ทิ้ง** — สัญญาคือ "ไม่มีผู้สมัคร = ระบบไม่ทำอะไร" ไม่ใช่ "ค้นหาต่อไปเรื่อยๆ ตราบใดที่ยังกดค้าง"
> การผูกที่ค้างอยู่ไม่ใช่ basis: การรอให้การเหวี่ยงจบขณะที่เป้าที่หายไปยังจัดอันดับไม่ได้ **คือจุดประสงค์ทั้งหมดของมัน** มันจึงรอดจากการใช้ทิ้งครั้งนี้ และตายที่ขอบฟ้าของตัวเอง หรือเมื่อมีการปล่อยปุ่มจริง

- **S4a `EnterEvidenceGap` + มีเจ้าของ → เข้า/อยู่ใน gap**
  ถ้าเพิ่งเข้า: เคลียร์ motion window (*"การเคลื่อนไหวที่เก็บใต้หลักฐาน Locked ฝากเข้า gap ไม่ได้"*)
  ย้าย `controller_context` → `gap_context` (ถ้ายังว่าง) · เก็บ echo ที่เสนอไว้ · ทิ้งสถานะ controller เชิงคาดการณ์ · health `EvidenceGap`
  *(Echo ถือ key เดิมไว้ตลอด gap; lifecycle เป็นเจ้าของ deadline 30 ms ตัวเดียว)*
- **S4b `EnterEvidenceGap` + gap หมดอายุ/ถูกปล่อย**
  บันทึก hard deadline (ถ้าหมดอายุ) · `record_lock_release("selection-gap-expired" | "selection-released")` · `NoCandidate` · ทิ้งสถานะการเล็งและการเคลื่อนไหว · block `TargetProof`
- **S4c `IdleNoAcquisition`**
  > เหตุผลในโค้ด: ไม่ได้เลือกอะไร ไม่ได้ถืออะไร — **เก็บ motion window ที่ท่าทาง On Motion อาจกำลังสะสมอยู่ไว้**

  ปล่อย lock · เก็บ echo · reset aim lock · เคลียร์ contexts · `Idle`
- **S4d `HardRelease` / `Proceed` / `EnterEvidenceGap` ที่ไม่มีเจ้าของ**
  > เหตุผลในโค้ด: การสูญเสียเชิงโครงสร้าง หรือ lock ที่ยังไม่ commit ซึ่งหลักฐานกลายเป็น Unknown — เคลียร์ Echo, lifecycle, contexts, reference และ motion window **พร้อมกัน** เพื่อให้ lifecycle Idle แปลว่า Echo lock เป็น None เสมอ

  `NoCandidate` · `release_preserving_capture_continuity`
  ถ้าเคยมีเจ้าของ: `record_lock_release("selection-structural")` · block `TargetProof`
  > เหตุผล: การสูญเสียเชิงโครงสร้างของเจ้าของที่ commit แล้วขณะที่ยังกดปุ่มค้างเป็นเรื่องชั่วคราว — การกดค้างยังจับเป้าต่อ **จึงไม่ล็อกการปล่อย**

  ถ้าไม่เคยมี: `Idle`
  → ทุกเส้นทาง **Sleep(Fast)**

**S5 — ไม่มี control proof ของเป้าที่เลือก**
block `TargetProof` · เคลียร์เป้า · `NoCandidate` · reset aim lock · เคลียร์ window **เว้นแต่กำลังรอ motion acquisition** · เคลียร์ contexts · ปล่อย lock → **Sleep(Fast)**

**S6 — ticket ของ generation ทรัพยากรหมดอายุ**
require_release · disarm diagnostics · ปลด activation floor · deactivate · unavailable `GameResources` · `NoCandidate` · release_for_external_invalidation
→ **EndSession(DormantRequireRelease)** ← ทางออกที่รุนแรงกว่าทุกตัวข้างบน

**S7 — ไม่มีตำแหน่งตาของผู้เล่นท้องถิ่น**
เคลียร์เป้า · `NoCandidate` · reset aim lock · เคลียร์ window (เว้นแต่ pending) · เคลียร์ contexts · ปล่อย lock · block `LocalIdentity` → **Sleep(Fast)**

### การตัดสินใจที่ไม่ใช่การปฏิเสธ

- `recoil_active = control_phase.delivers_spray_recoil()`
  > เหตุผล: นัดเดียวที่รับรู้แล้วเป็นธรรมชาติโดยเจตนา **เฉพาะนัดที่สองที่ยืนยันแล้ว**จึงให้อำนาจ output ของแรงสะท้อน แล้ว ramp ถึงช่วยกระสุนสเปรย์ถัดๆ ไปโดยไม่ต้องเดาจากปุ่มยิงที่กดค้าง
- **predicate การจับเป้า** — ไม่ถูกบล็อกโดย intent **และ**
  - `OnActivation` → ผ่านทุกตัว (*ปุ่มที่กดค้างคือเจตนาที่ยืนอยู่: ทุกผู้สมัครที่ crosshair ของผู้เล่นเองพาเข้ามาใน FOV ได้รับอนุญาต และการปล่อยปุ่มถอนทั้งหมด*)
  - `OnSupportingMotion` → intent ต้องอนุญาต (*ท่าทางอนุญาตเฉพาะผู้สมัครที่มันเคลื่อนเข้าหา และ basis ที่ผูกกับผู้สมัครแล้วจะเสนอตัวเดิมนั้นซ้ำพอดี*)
- **ปักหมุด basis กับสิ่งที่มันเลือก** เพื่อให้การแข่งกันเรื่องความสดลองผู้สมัครเดิมซ้ำ **แทนที่จะจัดอันดับใหม่ไปเจอคู่แข่ง**
- `pending_motion_acquisition` = OnSupportingMotion ∧ ¬had_committed_lock ∧ lock_changed
- `context` = เป้า · รูปทรง · proof · **aim_revision** (ไม่ใช่ revision รวม) · calibration_revision · calibration_bits
  > เหตุผล: ความต่อเนื่องของการแก้ขึ้นกับการตั้งค่า Aim ไม่ใช่การแก้ ESP / Trigger / เมนูที่ไม่เกี่ยว ส่วนอำนาจเรื่องควันถูกติดตามแยกและบังคับขอบการปล่อยปุ่มจริงเมื่อมันเปลี่ยน
- `proposed_control_epoch` = +1 ถ้า context เปลี่ยน
- **`proposed_authority` เป็นข้อยกเว้น ไม่ใช่ประตูจับเป้าที่สอง**
  > เหตุผลในโค้ด (ยาวและสำคัญ): มันกันไม่ให้หางของท่าทางที่กำลังจับเป้าถูกอ่านเป็นการแทรกแซงของผู้เล่นในภายหลัง **การจัดฉากไม่สำเร็จเสียแค่ข้อยกเว้นนั้น ไม่เสียอย่างอื่น** หางจึงถูกตัดสินตามปกติ — ซึ่งเป็นทิศทาง fail-closed เพราะผู้เล่นเก็บอำนาจไว้
  > การเพิกถอนการจับเป้าแทนทำให้สองประตูขัดกันเอง: basis ที่ผูกกับผู้สมัครแล้วอนุญาตการลองใหม่โดยไม่รันการทดสอบทิศทางซ้ำ ขณะที่การจัดฉากรันซ้ำกับมือที่อาจหยุดไปแล้ว **แล้วรื้อ lock ทิ้งเพราะเรื่องนั้น**

---

---

# ผลสรุป

## ขนาดจริงของการปฏิเสธ

**61 จุดออกจาก tick** ใน 9 ไฟล์

| ไฟล์ | จุดออก |
|---|---|
| activation.rs | 13 |
| lifecycle_capture.rs | 11 |
| readiness.rs | 8 |
| retained_target.rs | 8 |
| selection.rs | 7 |
| gate_commit.rs | 5 |
| proposal.rs | 5 |
| authorization.rs | 4 |

### การปฏิเสธหนึ่งครั้งไม่ใช่ "เหตุผล" — มันคือ 5 มิติ พร้อมสูตรรื้อ

| มิติ | จำนวนค่า |
|---|---|
| `AimSessionOutcome` (ความรุนแรงระดับ session) | 5 — Dormant · DormantRequireRelease · DormantRequireExplicitRearm · Reattach · Shutdown |
| `AimTickSchedule` (จังหวะที่จะกลับมา) | 5 — Immediate · CapturePending · Fast · Inactive · Attach |
| `AimBlockReason` (สุขภาพที่ผู้ใช้เห็น) | 20 |
| `SelectionReleaseCause` (ทำไมปล่อยเป้า) | 11 |
| `SessionEndCause` (ทำไม session จบ) | 14 |
| `StagedCaptureCause` (การ capture ล้มเหลวที่ไหน) | 21 |

**+ สูตรรื้อสถานะเฉพาะตัวของแต่ละจุด**

---

## 🔴 สองสิ่งที่ยากที่สุดในโค้ดนี้ เป็นผลข้างเคียงของการกลายพันธุ์ทั้งคู่

### หนึ่ง — สูตรรื้อ (teardown)

61 จุดปฏิเสธ แต่ละจุดเคลียร์สถานะคนละชุด `exits.rs` แชร์ได้แค่ 3 สูตร ที่เหลือเขียนสดและไม่ซ้ำกัน

**เพราะโค้ดกลายพันธุ์ `state.control_session.*` ไประหว่างทาง จึงต้องย้อนตอนปฏิเสธ**

### สอง — การย้อนกลับ (rollback) ← อันนี้หนักกว่า

ตัววางแผน RCS **เดินหน้าไปแล้วตั้งแต่ก่อน controller ทำงาน** ทุกเส้นทางที่สุดท้ายไม่ได้ส่งแพ็กเก็ต ต้องเรียก `rollback` คืน

มีจุด rollback หลายจุด และแต่ละจุดมีคอมเมนต์ยาวอธิบายว่าทำไมมันถึงจำเป็น ตัวอย่าง:

> *"tick นี้ไม่พาแพ็กเก็ต ดังนั้นขั้น punch ที่ตัววางแผนบริโภคไปตอนอนุญาตก่อน tick จึงไม่เคยถูกส่ง เอามันคืน แบบเดียวกับเส้นทาง output ที่ถูกงดข้างล่าง — **การย้อนกลับตรงนั้นอยู่ห่างออกไป 50 บรรทัดและครอบไม่ถึงตรงนี้ ซึ่งเป็นเหตุผลที่เงื่อนไขที่มันคุ้มอยู่เป็นจริงเสมอ**"*

คอมเมนต์ที่ต้องอธิบายว่า "เงื่อนไขนี้เป็นจริงเสมอ" คือกลิ่นของโครงสร้างที่ผิด

### ทั้งคู่หายไปเองภายใต้ seam

```
กลายพันธุ์:  ปฏิเสธ → ต้องเคลียร์ 4 อย่าง    ·  ไม่ส่ง → ต้อง rollback ตัววางแผน
บริสุทธิ์:   ปฏิเสธ → คืน successor ที่ไม่มี  ·  ไม่ส่ง → ไม่รับ successor เข้า
```

**นี่คือเหตุผลที่หนักที่สุดที่พบว่า Echo คุ้ม** — และมันไม่ใช่เรื่อง "อ่านง่ายขึ้น" แต่เป็นการลบคลาสของบั๊กทิ้งทั้งคลาส

---

## ✅ สิ่งที่ผมเสนอไว้แล้วพบว่า "มีอยู่แล้ว"

**ข้อ A — recoil วางแผนก่อน controller** ไม่ใช่ข้อเสนอ มันคือสิ่งที่โค้ดทำอยู่ และ**เหตุผลถูกบันทึกไว้พร้อมความล้มเหลวที่เคยเกิด**:

> *"สองอย่างผิดพลาดตอนที่พับมันเข้ามาหลัง controller ครั้งแรก: error ของ controller มีขั้น punch อยู่แล้ว การยกเลิกขั้นนั้นในแพ็กเก็ตเดียวกันจึงชดเชยสองเท่าและ tick ถัดมาเหวี่ยงกลับ (double compensation) และตัววางแผนยังไล่ตามการสลายตัวของสปริงกล้องระหว่างนัดด้วย (ฟันเลื่อยที่อัตราการยิง) รวมกันแล้วผู้รายงานเห็นภาพสั่น"*

**ต้องยกข้อความนี้เข้า Echo แบบคำต่อคำ** — มันคือบันทึกการหักล้างที่ §3 ของ REWRITE.md บอกว่าหายง่ายที่สุด

---

## ❌ สิ่งที่ผมเสนอไว้แล้วพบว่าผิด

### "composer" ไม่ใช่โมดูล — การประกอบจริงคือ 3 บรรทัด

```
final = (aim_counts + feedforward_counts)  แล้วผ่านประตู checked_zero
```

เพราะกฎการประกอบไม่ได้อยู่ในตัวประกอบ แต่**กระจายไปอยู่ในเจ้าของที่ถูกต้องอยู่แล้ว**:

| กฎ | อยู่ที่ไหนจริง |
|---|---|
| มือจ่ายหนี้ recoil ก่อน | **ข้างในตัววางแผน RCS** — `physical_mouse_total` เป็น input ของแผน ไม่ใช่การลบทีหลัง |
| ยอมมือตามสัดส่วน | **ข้างในตัวควบคุม aim** — `player_intent` + `player_conflict_weight_milli` เป็น input ของ `AimLockTick` |
| ลด gain ในตัวเป้า | **ข้างในตัวควบคุม aim** — ผ่าน `AimLockState::BodyHolding` |
| barrier / checked-zero | ประตูเดียว 1 บรรทัดที่ท้ายสุด |

**การย้ายกฎพวกนี้ออกมาไว้ใน composer จะทำให้สถานะของตัววางแผนถูกฉีก** — เครดิตของมือเป็นสถานะที่มีขอบเขตต่อหนึ่งชุดยิง มันต้องอยู่กับตัววางแผน

**ปัญหาจริงไม่ใช่การประกอบ — มันคือความเป็นธุรกรรม (transactionality)**

---

## 🔑 สถานะที่ถูกพกข้าม tick — ช่องว่างที่ใหญ่ที่สุดที่ผมเคยบอกไว้ ตอนนี้มีรายการแล้ว

นี่คือสิ่งที่ replay corpus ต้องครอบ และเป็นสิ่งที่ธุรกรรมต้อง commit-หรือ-ไม่รับ ทั้งก้อน

| สถานะ | ประกอบด้วย |
|---|---|
| **Aim lock** | 7 สถานะ (Idle · Acquiring · Tracking · BodyHolding · Holding · SuspendedInput · SuspendedVisibility) + นาฬิกา + อัตลักษณ์/generation ของ Pull Episode + **Carried Momentum (ความเร็ว + ความเร่ง)** |
| **Lifecycle** | 10 การเปลี่ยนผ่าน + deadline 30 ms ของ Evidence Gap + Kill Cooldown |
| **RCS lane** | หนี้ที่ค้าง (`pending_deg`) + เครดิตของมือ + ฐานที่ถือไว้ (`held_deflection`) + สถานะต่อชุดยิง |
| **Spray recoil** | phase · นับนัดล่าสุด · อาวุธ · punch นัดแรก · เศษที่พก |
| **Spray delivery** | ความต่อเนื่อง + punch ที่มีผล |
| **Intent basis** | เป้า/รูปทรงที่ผูก · เซตที่ถูกบล็อก · หน้าต่างท่าทาง |
| **Motion authority** | หน้าต่าง · baseline · ตัวจำแนก |
| **Contexts** | controller_context · gap_context · correction_reference |
| **Epochs** | control_epoch · reinitialization_epoch · degrees_per_count |
| **Press** | ลำดับการกดของตัวสร้าง |
| **ตัวล็อกความผิดพลาด** | controller_fault · unsupported_weapon_latched · input_error_latched |
| **ที่สังเกตไว้** | authority revisions · output ticket |

**~15 ก้อน** และทุกก้อนต้อง commit พร้อมกันหรือไม่ commit เลย

---

## 🔴 พฤติกรรมสวนสัญชาตญาณที่บันทึกไว้ — ห้ามให้หาย

> **หมุน RCS Vertical ลงเพื่อแก้อาการดึงเกิน แล้วมันแย่ลง**

เหตุผลในโค้ด: เครดิตของมือสะสมแบบ**ไม่ถูกสเกล** ขณะที่การลงบัญชีที่ขอบนัดซึ่งบริโภคเครดิตนั้น**ถูกคูณด้วย `vertical_fraction`** ที่ Vertical = 0 (ซึ่งยัง `runtime_demand()` อยู่ เพราะเงื่อนไขคือ `enabled && (vertical > 0 || horizontal > 0)`) **ไม่มีอะไรบริโภคมันเลย** ค่าจึงค้างอยู่ที่พื้นตลอดทั้งชุดยิง

นี่คือความรู้ที่ไม่มีทางได้กลับมาจากการอ่านโค้ดที่มันผลิตขึ้น

---

## กฎที่ตายแล้ว / โค้ดที่ตายแล้ว — สรุปทั้งหมดที่เจอ

| # | สิ่งที่ตาย | อยู่ที่ไหน |
|---|---|---|
| 1 | กฎ Aim v63 สามข้อ | CONTEXT.md (สามหัวข้อ) |
| 2 | ชื่อ `Target Body Player-Control Zone` | glossary + trace + ชื่อ test |
| 3 | คอมเมนต์ที่บอกว่าปุ่มเมาส์ซ้ายใช้กับ Aim ไม่ได้ | ค่าคงที่สำรองของปุ่ม |
| 4 | คอมเมนต์ *"post-shot phases never target-steer"* | selection.rs |
| 5 | **กิ่ง `!allows_target_acquisition()`** — โค้ดตาย + วินิจฉัย 9 ฟิลด์ที่ไม่เคยยิง | selection.rs |
| 6 | **spray delivery lane** — คำนวณแพ็กเก็ตที่ไม่มีใครส่งตั้งแต่ aim-v69 | proposal.rs / recoil.rs |

**รูปแบบเดียวกันทั้ง 5 และ 6:** กฎเปลี่ยน → predicate ถูกลดรูปเป็นค่าคงที่ → กิ่งใต้มันตาย → compiler ไม่เตือนเพราะยังเข้าถึงได้ในทางไวยากรณ์

**ดัชนีที่ถูกที่สุดในการจับ: ตัวนับที่ไม่เคยขยับใน capture จริง**

---

# 🔑 ข้อค้นพบที่สำคัญที่สุด — โค้ดค้นพบรูปทรงที่ถูกแล้ว แต่ใช้แค่ที่เดียว

`final_transaction/rejection.rs` มี type นี้:

```
FinalRejectionDisposition {
    basis_outcome          สิ่งที่เกิดกับ basis ของท่าทาง
    diagnostic_stage       ขั้นไหนของการอนุญาตที่ล้มเหลว
    control_reason         สตริงที่ไปโผล่ใน trace
    recovery               ← สูตรฟื้นตัว แบบมี type 5 แบบ
    visibility             หลักฐานการมองเห็นที่แนบมา
}

FinalRejectionRecovery =
    RetryCommittedLock
  | SuspendCommittedVisibility(สถานะการมองเห็น)
  | RequireActivationRelease
  | ObserveCommittedEvidence(หลักฐาน)
  | ReleaseSpeculativeTarget
```

doc ของมันเขียนไว้ตรงๆ:

> **"ทำให้ผลลัพธ์เชิงนโยบายทุกอย่างของการพิสูจน์สุดท้ายที่ล้มเหลว เป็นมาตรฐานเดียวกัน"**
> *"worker ที่ attach อยู่ยังรับผิดชอบการกลายพันธุ์ lifecycle และ controller เอง การเก็บการจำแนกไว้ที่นี่แปลว่า **การปฏิเสธการพิสูจน์แบบใหม่จะไม่สามารถมีความหมายต่างกันในแต่ละจุดเรียกได้**"*

**นี่คือหลักการเดียวกับที่ผมพยายามเสนอมาสามรอบ — และมันถูกค้นพบ เขียน และพิสูจน์ในโค้ดนี้ไปแล้ว**

## แต่มันถูกใช้กับ **1 phase จาก 8**

`authorization.rs` เป็นไฟล์เดียวที่มี type นี้ อีก ~57 จุดปฏิเสธจำแนกตัวเองสดๆ ที่จุดเรียก — **ซึ่งคือสิ่งที่ doc ของ type นี้บอกว่าห้ามเกิด**

ไม่ใช่เพราะใครประมาท แต่เพราะ **ไม่มี type เทียบเท่าสำหรับ phase อื่น**

## ดังนั้นข้อเสนอสถาปัตยกรรมของ Echo ไม่ใช่การประดิษฐ์ แต่คือ

> **เอา `FinalRejectionDisposition` ไปใช้กับทั้ง 61 จุด**

ซึ่งเป็นข้อเสนอที่แข็งกว่า "phase = การพับ" มาก เพราะ:

- รูปทรงถูกพิสูจน์แล้วในโปรดักต์นี้เอง ไม่ใช่ของนำเข้า
- เหตุผลถูกเขียนไว้แล้ว ไม่ต้องเถียงใหม่
- ขอบเขตของงานชัด: 1 phase ทำแล้ว เหลือ 7

**และเมื่อรวมกับ successor ที่บริสุทธิ์ สูตรฟื้นตัวที่มี type ก็กลายเป็น successor ที่มี type** — สองข้อค้นพบมาบรรจบกันพอดี

---

# วงเล็บมีสองชั้น ไม่ใช่ชั้นเดียว

ผมเคยเขียนว่า "พิสูจน์ → คำนวณ → พิสูจน์ใหม่ → ส่ง" นั่นคือ**วงเล็บของการพิสูจน์** แต่ `stage_tick` เผยว่ามีวงเล็บที่สองซ้อนอยู่

## วงเล็บของมือ (motion bracket)

```
เปิด:    consume(raw) → expire_window → ประเมิน gap หรือ locked
              ↓
         [ staged capture — การอ่านเกมที่แพงที่สุดของ tick ]
              ↓
ปิด:     consume(raw) → expire_window → ประเมิน gap หรือ locked   (ตรรกะเดียวกันเป๊ะ)
```

**มือถูกอ่านสองครั้ง คร่อมการ capture** เพราะมือขยับได้ระหว่างที่เรากำลังอ่านเกม

และถ้าวงเล็บเปิดตัดสินว่ามือกำลังฝืน → **งดทั้ง tick ทันที ไม่ต้องไปอ่านเกมให้เปลือง**

```
if motion_yields_output(opening_motion) → Sleep(Fast)
```

**"มือชนะเสมอ" ไม่ได้อยู่ในตัวประกอบตอนท้าย มันเป็นประตูบานแรกก่อนงานที่แพงที่สุดจะเริ่ม** — ทั้งถูกทั้งเร็ว

## `prepare_control_observation` ถูกเรียกสองครั้งด้วยเหตุผลเดียวกัน

1. **ก่อน** การสแกน flash/model — *"revoke การจับเป้าก่อนยิงและความพร้อม Safe Entry ทันทีที่ตัวอย่างท้องถิ่นและ slot ตรงกัน **ก่อน**ที่การสแกนซึ่งแพงกว่าจะทิ้ง Safe Entry proof ที่ยังใช้ได้ไว้ ทั้งที่เกมรับรู้นัดยิงไปแล้ว"*
2. **หลัง** — *"ใช้ซ้ำเป็นตัวดักการแข่งกัน: นัดที่ถูกสังเกตระหว่างการสแกนโมเดลยัง revoke Safe Entry ได้ก่อนการเคลื่อนไหวนั้นจะถูกตัดสิน"*

**หลักการ: หลักฐานความปลอดภัยที่เดินหน้าทางเดียว ต้องถูกใช้ทั้งก่อนและหลังทุกช่วงที่นาน**

---

# 🔑 คำตอบของ "ทำไมรู้สึกว่ามันไม่ค่อยจับ" — บั๊กจริง วินิจฉัยแล้ว แก้แล้ว

คอมเมนต์ใน `stage_tick`:

> *"Player Override ที่ **ขอบของการกด** ถูกเอาออกแล้ว การจำแนกการเคลื่อนไหวที่ผู้เล่นกำลังทำอยู่ว่าเป็นการแย่งคืน แปลว่าการกดขณะที่มือกำลังขยับ — **ซึ่งคือสิ่งที่ผู้เล่นทุกคนทำ** — ข้ามทั้ง tick และไม่เกิดการดึงเลย*
> *log แสดงว่ามันยิงประมาณ **6 ครั้งต่อการเล่นทุก 30 วินาที** และมันคือรายงาน **"กดแล้วไม่มีอะไรจับ"***
> *ระหว่าง lock ที่ตั้งมั่นแล้ว การขัดแย้งโดยเจตนาถูกชั่งน้ำหนักทีละ tick ใน transition แทน (A2)"*

**นี่คือห่วงโซ่ อาการ → การวัด → สาเหตุ → การแก้ ที่ครบสมบูรณ์** และเป็นสิ่งที่ §8 บอกว่าหายง่ายที่สุด

*(ตอบคำถามที่ผมถามผู้ใช้ไว้เองสองรอบก่อน: ถ้าความรู้สึก "มันไม่ค่อยช่วย" มาจากช่วงก่อน aim-v84 นั่นคือบั๊กตัวนี้ และมันถูกแก้ไปแล้ว)*

---

# `stage_tick` — จุดปฏิเสธ

**L1–L3 — วงจรชีวิตสั่งจบ tick ตั้งแต่ต้น**
- `KillCooldown` → baseline · เคลียร์ events · ทิ้งสถานะการเล็ง · health KillCooldown
- `CooldownFinished` → baseline · เคลียร์ · Idle
- `Released` / `EvidenceGapExpired` → บันทึก hard deadline ถ้าหมดอายุ · `record_lock_release("evidence-gap-expired" | "lifecycle-released")` · `LifecycleReleased` · ทิ้งสถานะการเล็ง · Idle
→ ทั้งสาม **Sleep(Fast)**

**L4 — มือฝืนที่วงเล็บเปิด** → health Locked → **Sleep(Fast)** *(ประตูก่อนงานแพง)*

**L5–L8 — ผู้เล่นท้องถิ่นไม่พร้อม** → `dormant_local_exit` → **EndSession(Dormant)**
- entity list เป็น null → block `LocalIdentity`
- local pawn เป็น null → `Idle` *(ไม่ใช่ error — แค่ยังไม่เกิด)*
- ตัวเองตายแล้ว → `Idle`
- ความมีชีวิตไม่รู้ → block `LocalIdentity` *(ไม่รู้ = ปฏิเสธ)*
- อ่าน local frame ไม่ได้ → block `LocalIdentity`
- resolve slot identity ไม่ได้ → block `LocalIdentity`

**L9 — อาวุธไม่รองรับ** → `exit_unsupported_weapon` → **EndSession(DormantRequireRelease)**
> *"การใช้อุปกรณ์เป็นสถานะ Aim ที่ไม่ผลิต output ต้องการการเปิดใช้งานใหม่ เพื่อให้ baseline ของมันทิ้งการเคลื่อนไหวตอนถือมีด/ระเบิด ก่อนการจับเป้าแบบ On Motion ครั้งถัดไป"*

**L10–L11 — หลักฐาน flash**
- `Suppressed` → `flash_block_exit` → **Sleep(Inactive)**
  > *"flash block คือหลักฐานความปลอดภัยชั่วคราว ไม่ใช่ขอบของท่าทางใหม่ ทิ้งทุกสถานะที่ผลิต output ได้ **แต่รักษา episode ที่ทำงานอยู่และ generation ของ input ไว้** เพื่อให้การกดค้างเดิมฟื้นได้บนเลนไม่ทำงานที่มีขอบเขต เมื่อตาบอดหาย"*
- `Unavailable` → `flash_evidence_unavailable_exit` → **EndSession(DormantRequireRelease)**
  > *"ข้อมูล flash ที่หายหรือผิดคือความล้มเหลวของหลักฐาน ไม่ใช่การกดทับ flash ตามปกติ ปลดระวาง session และบังคับท่าทางใหม่ **เพื่อให้ UI รายงานว่า input ความปลอดภัยใช้ไม่ได้ แทนที่จะรายงาน Ready/Idle ที่ทำให้เข้าใจผิด**"*

**L12–L14 — drain Raw Input ที่วงเล็บปิด** → Pending / Changed / Unavailable-Poisoned → **CapturePending / Immediate / EndSession**

---

# `authorize_final` — จุดปฏิเสธ (typed)

ทุกการปฏิเสธผ่าน `FinalRejectionDisposition` แล้ว worker นำสูตรไปใช้:

- **`SuspendCommittedVisibility`** — เจ้าของที่ commit แล้วหลุดการมองเห็น
  - ถ้าตัดสินแล้วเป็น `Idle` → *"การมองเห็นหมดลงบนเจ้าของที่ commit แล้วขณะที่ยังกดค้าง: **การสูญเสียชั่วคราว ไม่ใช่ถาวร** การผูกที่ค้างอยู่ให้การกดเดิมจบงานได้เมื่อเป้าพิสูจน์ได้อีก — **ไม่ล็อกการปล่อย**"*
  - ไม่งั้น → ย้าย context เข้า gap · health Locked
- **`retrying`** → health `EvidenceGap` หรือ `Locked` → Sleep(Fast) *(ลองใหม่ tick หน้า)*
- **`RequireActivationRelease`** → require_release · เคลียร์ echo/gap/window · **reset ทั้ง session** · reset aim lock · reset spray → block `FinalAuthorization`
- **`ObserveCommittedEvidence`** · **`ReleaseSpeculativeTarget`** · **`RetryCommittedLock`**

---

---

# `commit_or_recover` — วงเล็บปิด

## 🔑 คลังคำ "สูตรฟื้นตัวแบบมี type" **ตัวที่สอง** — และมันปิดคำถามที่ค้างอยู่

ประตูส่ง output มีคลังคำของตัวเอง และมัน**ใหญ่ที่สุดในระบบ**

```
GateReject — 22 สาเหตุ  [อยู่ที่ชั้นแพลตฟอร์ม = ใช้ร่วมกันทั้ง Aim · Trigger · Nade]

  ExpiredEvidence · MotionSequenceChanged · InputCapturePending
  InputCaptureUnavailable · GateUnavailable · PlayerOverrideLatched
  ActivationLost · ForegroundOrMenuLost · SettingsChanged · SessionChanged
  ResourceGenerationChanged · ResourceLeaseExpired · AimPriority
  PhysicalMousePriority · NadeSessionActive · OutputOwnershipChanged
  ControllerClock · ProofInvalid · CommandViewChanged · LocalControlChanged
  WeaponChanged · VisibilityProofChanged
                          │
                          ▼  ยุบลงเหลือ
AimGateRecovery — 5 สูตร

  ImmediateRetry · BoundedYieldRetry · RetryExpiredEvidence
  RetryFreshEvidence · RequireActivationRelease
```

## ✅ การทดสอบที่ผมตั้งไว้ว่าจะฆ่าสถาปัตยกรรม — **ผ่าน**

ผมเขียนไว้ว่า *"ถ้าสถานะปลายทางของ 61 จุดไม่ยุบเป็นชุดที่จำกัด ชั้น 2 ตาย — ได้ 5–8 แบบ
สถาปัตยกรรมยืน ได้ 40 แบบ คือความคิดเพ้อฝัน"*

**โปรดักต์เดิมรันการทดสอบนี้ไปแล้วสองครั้ง โดยไม่รู้ตัว:**

| ขอบเขต | สาเหตุ | สูตรฟื้นตัว | อัตราส่วน |
|---|---|---|---|
| การอนุญาตขั้นสุดท้าย | หลายแบบ | **5** | — |
| **ประตูส่ง output** | **22** | **5** | **22 : 5** |

**อัตราการยุบคือ 22 → 5** ตรงกลางช่วงที่ผมเดาไว้พอดี

## และนี่คือหลักฐานที่แข็งที่สุดของทั้งการเก็บเกี่ยว

> **โปรดักต์นี้ค้นพบรูปแบบ "การปฏิเสธที่พกสูตรฟื้นตัวแบบมี type" มาแล้ว *สองครั้ง อิสระจากกัน*
> ที่ขอบเขตสองจุดที่การปฏิเสธยากที่สุด**

ไม่ใช่ความบังเอิญ และไม่ใช่ของนำเข้าจากที่อื่น — **มันคือรูปทรงที่ปัญหานี้บังคับให้เป็น**

สิ่งที่ไม่เคยเกิดคือการ**ทำให้มันเป็นกฎทั้งระบบ** อีก ~57 จุดจึงยังจำแนกตัวเองสดที่จุดเรียก

**หลักฐานยืนยันอีกชั้น:** ทั้งสองคลังคำมีสูตรชื่อ `RequireActivationRelease` เหมือนกัน —
**แนวคิดเดียวกัน ถูกนิยามซ้ำสองที่** เพราะไม่มีคลังคำกลาง

---

## จุดปฏิเสธของวงเล็บปิด

**G1 — นาฬิกาของตัวควบคุมเดินถอยหลัง** (ทั้งก่อนและหลังประตูยอมรับ)
> *"อย่ารับสถานะที่เสนอไว้: นาฬิกาที่ไม่เดินหน้าทางเดียวไม่ควรเกิดขึ้นเลย และเมื่อมันเกิด
> **ให้ทิ้งความคืบหน้าเชิงคาดการณ์ทั้งหมด แทนที่จะไว้ใจนาฬิกาที่สอบตกการตรวจ**"*

ตั้ง `ControllerFault::Clock` → ครั้งถัดไปต้อง **rearm อย่างชัดเจน** ไม่ใช่แค่ปล่อยปุ่ม

**G2 — ติดตั้งขอบเขตลำดับของจุดอ้างอิงไม่สำเร็จ**
> *"ประตูที่เรียงลำดับยอมรับ floor นี้ไปแล้ว ความล้มเหลวจึงเป็นไปไม่ได้ เว้นแต่สัญญาของ
> การ capture เปลี่ยน **คงความ fail-closed ไว้ในโปรดักชัน แทนที่จะรับจุดอ้างอิงที่ขอบเขต
> ลำดับของมันไม่ได้ถูกติดตั้ง**"*

← นี่คือการเขียนโค้ดรับเหตุการณ์ที่เชื่อว่าเป็นไปไม่ได้ ในทิศทางที่ปลอดภัย

**G3 — ประตูปฏิเสธ** → ยุบเข้า 5 สูตร แล้วแต่ละสูตรมีการรื้อของตัวเอง

**G4 — เป้าตายหลังการตรวจครั้งสุดท้าย** ← การอ่านเพิ่มแบบมีเงื่อนไข

เกิดเฉพาะเมื่อ *(มีเจ้าของที่ commit แล้ว)* **และ** *(เหตุผลที่ปฏิเสธคือหลักฐานไม่ผ่าน)*
เท่านั้น — อ่านสถานะเป้าเพิ่มหนึ่งครั้งเพื่อแยก **"เป้าตายจริง"** ออกจาก **"หลักฐานแข่งกัน"**

ถ้าตายจริง → เข้า **Kill Cooldown** *(ไม่ใช่การปฏิเสธ แต่เป็นการจบงานสำเร็จ)*

**เป็นตัวอย่างที่ดีของการอ่านที่แพงแต่คุ้ม: จ่ายเฉพาะตอนที่คำตอบเปลี่ยนการตัดสินใจจริง**

---

## หลักฐานมีอายุ 10 มิลลิวินาที

`evidence_expires_at = เวลาที่เริ่ม capture + 10,000 ไมโครวินาที`

สองเท่าของงบหนึ่ง tick **ข้อเสนอที่แก่เกินนี้ถูกปฏิเสธด้วย `ExpiredEvidence` → ลองใหม่ทันที**
และโค้ดระบุชัดว่าการหมดอายุ *"บอกแค่ว่าข้อเสนอนี้แก่เกินกำหนด มันไม่ได้ขาดหลักฐานเป้าหรือ
การมองเห็น และ**ห้ามสร้าง Evidence Gap หรือแตะพลวัตที่ commit ไปแล้ว**"*

← **การปฏิเสธต้องไม่ลงโทษเกินกว่าเหตุ** เป็นหลักการที่ควรยกเข้า Echo
