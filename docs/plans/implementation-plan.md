# Kế hoạch triển khai OpenGit

Ngày lập: 2026-10-24. Mục tiêu hiện tại `0.1.0`, trạng thái **Planned**, change class `none`. Đây là kế hoạch, không phải app đã triển khai hoặc release đã hoàn tất. [Roadmap](../roadmap.md) giữ phạm vi sản phẩm R1–R6; [kiến trúc](../architecture.md) giữ invariant; [0.1.0](0.1.0.md) là kế hoạch chi tiết đầu tiên. Không cài tool, tạo scaffold hay phát hành trong task planning này.

## Trình tự phiên bản và exit gates

| Phiên bản / yêu cầu | Epic theo thứ tự | Exit gate phải có evidence |
| --- | --- | --- |
| 0.1.0 — R1, R6 | Readiness/contracts → desktop scaffold → IPC/Git/process → registry/SQLite → inspection → scheduler/journal → local demo + failure harness → GUI auth → transactional clone → clone/task UX → acceptance/review | A1, A6 và F1–F5: open/recents nhiều repo; clone HTTPS/SSH public/private và local được cấp phép; lỗi/progress/cancel, destination/restart an toàn; build/launch thực tế. Private auth chưa có fixture thì gate bị BLOCKED |
| 0.2.0 — R2, R6 | Query DAG/parent identity → layout/pagination → Canvas + TanStack Virtual/sidebar → commit/diff Monaco tải khi cần → history/ref search → Hide/Solo nhiều branch → persist/refresh/selection | A2: merge-heavy fixtures đúng ancestry qua filter/page; shared ancestors giữ đúng; search ngoài loaded rows/filter; state/reopen/scroll ổn định. Đo baseline trên máy/dataset tham chiếu rồi mới chốt threshold |
| 0.3.0 — R3, R6 | Preflight/locks → fetch/prune → branch/tracking/tag → stage hunk/line, commit/amend/hook/signing → stash/conflict tối thiểu → push/upstream + FF-only pull | A3: workflow branch→stage→commit→push→tag trên remotes biệt lập; hook failure/rejected push/signing/conflict hiển thị đúng; divergence được giải thích; không silent chọn merge/rebase |
| 0.4.0 — R4, R6 | Merge/pull strategies → ordered cherry-pick/mainline → interactive rebase → resolver base/both/result và binary/rename/delete → continue/abort/skip → restart/reflog/revert | A4: clean/conflict/restart tại merge/rebase/cherry-pick pass; giữ công việc có sẵn; không hứa undo mọi tình huống; nếu force push được mở thì explicit intent + force-with-lease |
| 0.5.0 — R5, R4, R6 | Source/destination selection → pin source OID/import objects qua app refs → preview/order/trial worktree → apply/conflict/recovery → source-result mapping/cleanup | A5: same-repo/multi-remote và separate-repo pass; kiểm tra dirty state, HEAD, branch ở worktree khác, shallow/missing objects/object format/empty commit; nguồn không đổi; result/recovery đích được chứng minh |
| 0.6.0 — R1, R2, R3, R6 mở rộng | LFS/private submodule auth → worktree/shallow management → signing/accounts/enterprise → saved views/shortcuts/command palette/action preview → large/binary/image diff → profile/cache/watchers/IPC | Ma trận compatibility và threshold đo thực tế pass; liệt kê host/repo chưa hỗ trợ, không suy rộng coverage |
| 1.0.0 — R1–R6 tích hợp | Chốt license → Windows installer/signing/update trust → settings/cache migration → diagnostics/log export lọc secret → auth/recovery/user docs → integrated acceptance | Mọi R1–R6 có evidence, không required finding mở; private clone→commit/push→conflict→restart/recovery→cross-repo pass; release cần ủy quyền riêng |

Mỗi phiên bản sau phải có checkpoint planning ngay trước implementation: đối chiếu phiên bản đã thực hiện, kết quả benchmark/compatibility, contract/migration và unknown mới; lập packet có ownership/dependency/test trước dispatch. Các epic tương lai không phải microtask cố định hoặc quyền triển khai toàn roadmap. Correctness, secret handling và recovery thuộc gate của từng phiên bản, không dồn đến 1.0.0. macOS/Linux là phạm vi tương lai riêng.

## Acceptance dùng trong packet

| ID | Kết quả phải chứng minh |
| --- | --- |
| A1 | 0.1.0 open/clone local + public/private HTTPS/SSH được cấp phép; auth/errors/progress/cancel; destination và nguồn an toàn |
| A2 | 0.2.0 graph/search/diff/Hide/Solo đúng, persist và refresh |
| A3 | 0.3.0 daily Git workflows, rejection/hook/conflict paths |
| A4 | 0.4.0 integration/conflict/recovery/restart |
| A5 | 0.5.0 cross-repo transfer, nguồn giữ nguyên và recovery đích |
| A6 | UX desktop responsive, focus/keyboard, selection/loading/error/action feedback và identity riêng; đo từ 0.2.0 |
| F1 | Toolchain/dependencies/scripts thực tế, build và launch có evidence |
| F2 | Typed IPC/process/parser/path/secret boundary và failure tests |
| F3 | Git authoritative, persistence/migration và registry an toàn |
| F4 | Scheduling/locks, cancel, ordered events, stale-read và restart reconciliation |
| F5 | Validation tích hợp và review độc lập trước completion/release gate |

## Phân bổ công việc và checkpoints

Kế hoạch 0.1.0 gồm 13 packet Must nhưng dự kiến 7–9 handoff implementation bằng cách tái sử dụng worker theo module liên quan, không tạo 13 agent mới. Một cách nhóm khi dependency đã sẵn: 01; 02; 03–04; 05–06; 07; 08; 09; 10–11; 12–13. Nhóm không cho phép bỏ dependency hay để worker tự dispatch. Orchestrator giữ task record; OG-010-02 giữ manifests/contracts; module owners nhận mọi sửa code thuộc phạm vi mình.

Thông thường một worker; tối đa hai subagent đồng thời theo policy, trần cấu hình 10 là giới hạn khác. Có thể chạy OG-010-07 (UI) cùng OG-010-08 (harness) sau 06 nếu orchestrator ghi lợi ích thời gian từ hai luồng độc lập đủ bù context/tích hợp; không chia sẻ quyền ghi. Kết hợp kết quả rồi kiểm tra tích hợp. Các packet parser/auth/recovery/clone dùng worker_complex `gpt-6.1-sol`/high; UI/scaffold thông thường dùng worker_standard Sol/medium. Luna/medium chỉ cho theme/docs cơ học sau khi có pattern. Review riêng bằng Sol/high; Astra/xhigh cần capability gap, hard unknown hoặc repeated failure có evidence.

Ba review gates dự kiến: local demo/foundation (01–08), auth + clone core (09–10), tích hợp (11–13). Reviewer không phải tác giả; required findings trả về owner, sửa theo ID rồi re-review. Đây là soft allocation, có thể điều chỉnh theo kết quả; không bỏ check bắt buộc để tiết kiệm. Sau hai lần thất bại với cùng cách giải quyết, dừng cách đó và replan; unresolved serious risk được escalation ngay có lý do.

Checkpoint chi phí trước mỗi batch và review: kiểm tra dependency, reuse evidence/context, actual/expected handoffs, blockers, nhu cầu model; sau batch ghi usage runtime nếu có. Hiện usage **chưa biết**, không có billing telemetry, không có budget mặc định. Không tuyên bố tiền hoặc tỷ lệ tiết kiệm khi chưa có measured usage, billing method và giá hiện hành. Nếu người dùng đặt cap sau này, giữ cap và báo phần chưa xong, không đánh dấu hoàn tất.

## Điểm bắt đầu tiếp theo

Dispatch OG-010-01 trong [kế hoạch 0.1.0](0.1.0.md), tái sử dụng evidence ngày 2026-10-24 từ [task planning](../tasks/003-application-planning.md). Xác minh readiness build và chốt contracts trước scaffold; GUI auth implementation không là prerequisite của scaffold. Không có fixture private được cấp phép nên live auth gate còn chặn nghiệm thu A1, không chặn hoàn thành planning hoặc các nhánh nền tảng đủ điều kiện.

Version chỉ thay đổi theo [versioning policy](../versioning.md). Kế hoạch hoàn tất, ứng dụng hoàn tất và release hoàn tất là ba kết quả khác nhau; không tạo commit/tag/push/publish khi chưa được ủy quyền.
