# Kiến trúc dự kiến của OpenGit

Ngày lập: 2026-10-24. Đây là quyết định thiết kế để triển khai, chưa phải mô tả ứng dụng đã chạy. Phạm vi sản phẩm do [roadmap](roadmap.md) quyết định; thứ tự triển khai nằm trong [kế hoạch tổng thể](plans/implementation-plan.md) và [kế hoạch 0.1.0](plans/0.1.0.md).

## Quyết định và ranh giới

Windows là nền tảng đầu tiên. Tauri 2 chứa React/TypeScript/Vite, Zustand và CSS variables; Rust/Tokio xử lý Git CLI, filesystem, điều phối và SQLite. Không cần HTTP server riêng hay tài khoản cloud của ứng dụng để clone. Canvas 2D/TanStack Virtual và Monaco tải khi cần bắt đầu ở 0.2.0. Phiên bản dependency, SQLite driver, cơ chế sinh type và công cụ E2E chỉ được pin sau preflight và scaffold tương thích thực tế.

Frontend gửi ý định nghiệp vụ; backend giữ quyền chạy process, nhận diện đường dẫn, điều phối và kiểm tra điều kiện. Không cung cấp API shell/fs/cwd/argv tổng quát cho frontend. Tauri capabilities giới hạn quyền của cửa sổ nhưng custom commands vẫn phải tự kiểm tra payload và phạm vi repository. Xem [Tauri capabilities](https://v2.tauri.app/security/capabilities/).

```text
src/
  contracts/                 # type IPC đã đóng băng
  features/workspaces/       # workspace, recents, lựa chọn repo
  features/repository/       # status, branch, identity
  features/tasks/            # trạng thái và lỗi tác vụ
  features/clone/            # nguồn, đích, auth, progress
  components/               # theme, focus, thành phần dùng chung
  stores/                   # trạng thái UI và projection từ backend
src-tauri/src/
  ipc/                      # command có type, validation, event/snapshot
  domain/                   # identity, model, contract
  git/                      # runner, parser, inspection
  repositories/             # registry, canonical identity, discovery
  operations/               # scheduler, locks, journal, reconcile
  auth/                     # HTTPS helper và SSH adapter
  clone/                    # staging, publish, recovery
  persistence/              # SQLite, workspace, migrations
src-tauri/migrations/
src-tauri/tests/
tests/e2e/
```

Đây là cây thư mục đề xuất; chưa tạo code. OG-010-01 chốt thiết kế contract; OG-010-02 là người ghi duy nhất manifests, lockfiles và type dùng chung. Chủ module trong các packet sở hữu implementation của module đó. Thay đổi contract phải được orchestrator chuyển về chủ sở hữu, đồng bộ Rust/TypeScript và kiểm tra compatibility trước khi dispatch phụ thuộc.

## Domain và IPC

| Model | Dữ liệu và bất biến |
| --- | --- |
| Workspace | ID, danh sách repository ID, selection; UI có thể đổi selection mà không đổi Git |
| Repository | ID, canonical common Git directory, danh sách worktree; aliases cùng identity được nhóm |
| Worktree | ID, repository ID, canonical root, HEAD/branch hoặc detached/unborn, operation state quan sát từ Git |
| Remote | Repository ID, tên, URL hiển thị đã lọc; không persist URL chứa credentials |
| Operation | ID, scope, kind, phase, sequence, progress, khả năng cancel, lỗi terminal có cấu trúc |

OID là định danh opaque theo object format, không cố định 40 ký tự. Backend giữ OS/raw-path identity; IPC chỉ truyền display và opaque handle, không round-trip đường dẫn bằng chuỗi lossy. Dialog OS chọn đường dẫn được backend đăng ký thành repo/worktree/clone handle; request tiếp theo dùng ID, backend kiểm lại quyền và phạm vi.

Outline command: đăng ký lựa chọn từ dialog; mở workspace/recents; đọc snapshot repo; yêu cầu clone bằng source đã validate và destination handle; cancel operation; lấy operation snapshot theo sequence. Response lỗi gồm code, phase, retry intent an toàn và thông báo đã lọc; không chứa raw stderr. Events mang operation ID/sequence; client phát hiện thiếu hoặc đảo thứ tự để resync snapshot. Mọi read gắn generation của selection, bỏ kết quả cũ khi đổi repo. OG-010-01 chốt schema/error taxonomy; OG-010-02 vật chất hóa type trước worker phụ thuộc.

## Git, đường dẫn và process

Git là nguồn sự thật cho HEAD, refs, working tree và thao tác đang dở. SQLite chỉ chứa workspace/recents, trạng thái UI, cache có thể vô hiệu hóa và operation journal. Migrations có transaction, giữ database khi lỗi/corrupt để chẩn đoán và phục hồi; không tự xóa DB để khởi động lại.

Runner gọi executable tin cậy bằng argument arrays, không shell-interpolate. Validate request/ref/path/scope trước spawn. Giới hạn dung lượng và thời gian đọc output không tin cậy; lọc trước khi đưa vào logs/UI/SQLite. Parser status dùng porcelain v2 với NUL delimiter, xử lý rename/unmerged và bỏ qua header chưa biết theo [định dạng Git status](https://git-scm.com/docs/git-status). Cần kiểm tra UTF/path lạ, output thiếu/malformed và Git không tìm thấy.

Discovery nhóm linked worktrees theo common Git directory ngay trong 0.1.0; quản lý đầy đủ worktree ở 0.6.0. Bare repo có thể đăng ký metadata ở chế độ chỉ đọc, không giả tạo status/action cần worktree; bare source vẫn clone bình thường. Nested repo chỉ được thêm khi người dùng chọn rõ, không tự quét đệ quy.

## Xác thực

Chấp nhận HTTPS, SSH/SCP và local source; phân biệt Windows drive path với SCP. Từ chối credential nhúng trong URL và custom remote helpers. Tôn trọng Git config, proxy, CA và SSH của người dùng; không tắt TLS/host verification, không sửa global `safe.directory` để che lỗi.

HTTPS dùng credential helpers/GCM và browser native; SSH dùng OpenSSH/agent. Hai luồng độc lập, không ép SSH qua GCM. [GCM](https://github.com/git-ecosystem/git-credential-manager/blob/main/README.md) là thành phần hỗ trợ HTTPS, sự hiện diện của nó không chứng minh private clone/SSO thành công. Phải chứng minh từ GUI không có terminal: encrypted-key passphrase, host-key prompt, browser cancel, credential hết hạn/quyền bị từ chối. Nếu cần trusted askpass broker, đóng băng threat model, IPC binding, ACL và lifetime bí mật trước implementation; prompt content/secret không được persist.

## Điều phối, cancel và khởi động lại

Conflicting writes khóa theo canonical common Git directory; khóa worktree bổ sung theo thứ tự cố định để tránh deadlock. Clone có destination reservation và giới hạn concurrency mạng. Khóa nội bộ không ngăn CLI/IDE bên ngoài: recheck preconditions ngay trước thao tác và refresh kết quả từ Git.

State machine: `Queued → Running ↔ WaitingForUser`; từ trạng thái đang hoạt động có thể sang `Cancelling`; kết thúc đúng một lần bằng `Succeeded`, `Failed`, `Cancelled` hoặc `Interrupted`. Events có sequence tăng; snapshot là điểm resync. Cancel chỉ dừng cây process Git thuộc operation, không giết shared SSH agent hay browser người dùng. Journal ghi ngữ cảnh, không làm nguồn sự thật Git. Restart đối chiếu Git/filesystem/process sở hữu, không tự resume mutation chưa chứng minh an toàn.

## Clone theo giao dịch

1. Validate source và đích, canonicalize ancestor, kiểm tra junction/reparse points và reserve destination; giới hạn root được phép.
2. Tạo sibling staging cùng filesystem với dấu ownership và journal operation; chạy clone vào staging, nhận progress có giới hạn và verify repository. Tùy chọn clone bám [Git clone](https://git-scm.com/docs/git-clone), không suy ra mọi transport/provider đều đã được kiểm thử.
3. Publish bằng filesystem operation **no-replace** chống race, chỉ khi destination còn vắng. Precheck/canonicalization đơn thuần không đảm bảo atomicity; OG-010-10 phải chứng minh primitive Windows và xử lý rename failure.
4. Đăng ký repository/idempotent completion. Chỉ cleanup staging đã xác minh app sở hữu; không recursive-delete destination của người dùng. Crash trước/sau publish được reconcile theo journal và filesystem.

Cancel có thể thắng trước atomic publish. Khi publish thành công, kết quả là Success dù cancel đến muộn; không xóa target đã publish. Kiểm tra sentinel ở đích, repo nguồn không đổi, restart sau publish trước registry và race do actor ngoài app là gate bắt buộc.

## Quyết định còn mở và rủi ro

| Gate / chủ trì | Chưa biết hoặc rủi ro | Điều kiện đóng |
| --- | --- | --- |
| OG-010-01/02 | Rust/MSVC, Node/package manager, SQLite driver, pins, type generation, E2E driver | Toolchain kiểm chứng, build/launch và reproducibility; xem [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) và [Vite guide](https://vite.dev/guide/) |
| OG-010-03/05 | Encoding, malformed output, process tree, missing binary | Fixture parser và boundary pass, output/secret có giới hạn |
| OG-010-06 | External Git, shared common directory, race terminal/restart | Lock/precondition/event/reconciliation tests pass |
| OG-010-09 | GUI auth, enterprise proxy/CA/SSO và fixture private chưa có | Adapter contract + threat model; live gate chỉ pass khi có truy cập được cấp phép |
| OG-010-10 | TOCTOU/junction, no-replace publish, disk/permission failures | Race/fault injection và destination sentinel pass |
| OG-010-12/13 | Coverage vượt evidence, logs rò secret, migration lỗi | Kết quả compatibility ghi rõ blocked/unsupported; review độc lập |

Các invariant trên là quyết định; khả năng môi trường/provider là giả định phải kiểm tra. Không cam kết mọi host đều tương thích. Ma trận lỗi và packet thực thi nằm trong [0.1.0](plans/0.1.0.md).
