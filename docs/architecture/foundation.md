# Foundation 0.1.0 — design freeze v1

OG-010-01, 2026-10-24; bổ sung [kiến trúc](../architecture.md), [IPC v1](ipc-contract.md) và [readiness](../testing/environment-matrix.md). Thiết kế đã chốt để triển khai; không phải bằng chứng runtime. Chỉ open/workspace/inspection/auth/clone/task; graph và mutations sau clone thuộc phiên bản sau. Owner 02 duy nhất ghi manifests/lockfiles/Rust+TS contracts; module owners yêu cầu wiring qua orchestrator.

## Invariants và lưu trữ

- Git/filesystem là nguồn sự thật. SQLite lưu workspace, repo/worktree opaque IDs, canonical/raw OS identities, selection và journal; không lưu credentials, auth prompt/response hoặc raw output. Canonical common Git directory nhóm alias/linked worktrees, raw path không round-trip qua display string. Nested repository chỉ đăng ký khi chọn rõ; bare chỉ metadata read-only.
- Registry transaction deduplicate canonical identity, cấp IDs ổn định, update selection/revision atomically. Canonical identity được kiểm lại trước inspection/mutation; unavailable entry giữ ID và báo lỗi, không chuyển sang repo khác do path bị thay. Khóa nội bộ không khóa Git CLI/IDE ngoài app.
- SQLite schema version riêng application version; migration transaction, rollback khi lỗi, giữ DB gốc khi corrupt/permission failure, không xóa hoặc tự tạo DB thay thế để che lỗi. Driver dự kiến `rusqlite` với `bundled`; owner 02 chốt exact pin cùng lockfile sau thử tương thích. Không thêm type generator.
- Runner dùng Git executable tin cậy, argv arrays và backend-scoped cwd; không shell interpolation. Đóng stdin khi không dùng, bound/time-limit streams; noninteractive inspection không mở auth. Không thay config global, `safe.directory=*`, TLS hoặc SSH verification. Parser porcelain v2 `-z` giữ NUL path records, rename/unmerged; unknown headers bỏ qua, malformed record lỗi an toàn.
- Conflicting writes serialize bằng commonGitDir key; nhiều keys lấy theo thứ tự canonical cố định, worktree key sau repo key. Clone reserve `(canonical parent identity, Windows case-insensitive leaf)` trước queue, tối đa 2 clone network active; không chỉ lock path string. Recheck external state ngay trước publish và inspection sau mutation.

## Lifecycle, journal và publish

`Queued → Running ↔ WaitingForUser`; từ ba state này có thể sang `Cancelling`; active state kết thúc bằng `Succeeded | Failed | Cancelled | Interrupted`. Terminal đúng một lần; `Cancelling → Succeeded` hợp lệ khi publish đã thắng. Queued cancel không spawn; waiting cancel invalidate prompt; progress không đổi state terminal. `Succeeded` chỉ sau publish và registry transaction; journal có thể chờ reconcile nếu crash/DB fail sau publish, tuyệt đối không báo Cancelled hoặc xóa đích đó.

Journal tối thiểu: operation ID/kind/state/phase/timestamps, validated transport kind + safe source display, canonical parent identity/raw path, destination leaf, uniquely owned sibling staging identity/marker nonce, publish intent/commit evidence và resulting repo/worktree IDs. Không lưu source credential/input URL, prompt, secret, stderr hoặc process env. Journal intent/staging ownership phải durable trước Git spawn; prepublish intent durable trước rename; registry+terminal commit transaction sau publish. Event chỉ sau journal commit, full snapshot theo IPC fence.

Clone tạo staging sibling cùng filesystem bằng tên ngẫu nhiên độc quyền, marker nằm ngoài Git checkout trong app-owned metadata (không để remote tạo/ghi marker giả). Chỉ cleanup staging khi identity + ownership marker + journal cùng khớp, process tree đã dừng; root guard chống reparse/junction, không recursive-delete qua path chưa chứng minh. Parent folder handle giữ stable OS identity, revalidate ancestry/reparse tại mỗi boundary. Backend dùng Windows handle-based no-replace rename (không fallback overwrite/copy/delete) và chứng minh primitive ở OG-010-10. Atomic rename là commit point, destination có sẵn/race ⇒ failure giữ sentinel. Đã publish thì không bao giờ cleanup destination.

| Failure / recovery | Quyết định và evidence phải có ở implementation |
| --- | --- |
| Validation/alias/scope sai | Reject trước spawn; cùng commonGitDir cùng lock; fixture linked/bare/nested/Unicode |
| Transfer/checkout/disk/permission failure | Safe Failed; owned staging cleanup hoặc giữ evidence cleanup failure; source/sentinel nguyên vẹn |
| Cancel trước publish | Dừng owned Git tree, chờ exit, cleanup ownership-verified staging; Cancelled đúng một lần |
| Cancel đua publish | Đồng bộ cancel gate với atomic publish; publish thắng ⇒ success/reconciliation, không xóa target |
| Crash trước publish | Reconcile journal + real filesystem; Interrupted, không auto-resume mutation; chỉ cleanup đã chứng minh owned |
| Crash sau publish trước registry | Chứng minh target đúng staging identity/commit evidence, verify Git rồi đăng ký idempotent + Succeeded; bất định ⇒ Interrupted, giữ target và báo kiểm tra thủ công |
| Actor ngoài app đổi parent/destination/Git | Stable handle + no-replace, recheck preconditions; ExternalChange/UnsafePath, không chiếm/xóa dữ liệu actor |
| DB fail/corrupt hoặc journal write fail | Không bắt đầu mutation mới khi thiếu durable intent; published target giữ nguyên, recovery không clone lại; transaction rollback |
| Lost/reordered events, stale reads | Full atomic snapshot/fence + stream sequence, generation và workspace revision; không tạo terminal từ UI |

## GUI auth threat model

HTTPS dùng configured Git credential helpers/GCM/browser; SSH dùng OpenSSH/agent. Tôn trọng proxy/CA/config, host verification và credentials có sẵn. GUI không có terminal phải kiểm chứng passphrase, host-key confirmation, browser cancel và denied/expired credentials; adapter proof thuộc 09, chưa thực hiện ở design freeze.

Nếu Git/OpenSSH cần askpass, main binary chỉ vào helper mode khi explicit internal marker + nonce + pipe binding hợp lệ; mặc định mở UI. Broker là Windows named pipe current-user-only ACL, reject remote clients, random per-process nonce, operation/process binding, one-time response và expiry 120 giây. Cùng-user process độc hại/admin nằm ngoài ranh giới bảo đảm; không mở TCP/local HTTP endpoint. Helper không được tự nhận operation ID không xác minh; prompt text không tin cậy chỉ phân loại thành template, unknown prompt fail an toàn. Không truyền secret bằng argv/env/journal/event; helper chỉ ghi response ra stdout riêng mà Git đọc, không logger. Response transient, minimize copies/zeroize nơi khả thi; không hứa xóa mọi OS/runtime copy.

`respond_auth_prompt` dùng one-time handle của [IPC](ipc-contract.md); secret không trả lại frontend và không persist. Cancel/restart/expiry đóng pipe, invalidate handle và dừng helper/Git descendants do app tạo; Windows Job Object/process ownership phải chứng minh, PID đơn thuần không đủ do reuse. Không kill browser, GCM phiên dùng chung hay SSH agent ngoài ownership. No host-key autoaccept, không ghi `known_hosts` thay người dùng ngoài luồng OpenSSH xác nhận. Mọi error/progress/log đi qua sanitizer trước sink, fallback code/message chung khi không đủ confidence.

Thiết kế này yêu cầu tests disposable cho state/race/restart, path guard, parser, prompt replay/ACL, canary redaction, DB rollback và publish no-replace. Không có mutation hoặc auth broker được chạy trong OG-010-01. Sources đã được orchestrator xác minh: [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), [typed Rust commands](https://v2.tauri.app/develop/calling-rust/), [Git status](https://git-scm.com/docs/git-status), [GCM](https://github.com/git-ecosystem/git-credential-manager/blob/main/README.md).
