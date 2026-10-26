# OpenGit

Desktop Git client ưu tiên cây commit trực quan, quản lý nhiều branch và Hide/Solo theo repository. Trải nghiệm lấy cảm hứng từ GitKraken, với thiết kế và tài sản đồ họa riêng.

**Trạng thái: đang triển khai 0.3.0.** GitHub `master` giữ baseline0.2.0 `ce5da58`; development ở `codex/0.3.0`. History/diff, commit thuộc nhánh nào, fetch/prune và private HTTPS/SSH đã có kiểm chứng native. UI branch/tag/upstream cũng đã qua source review và luồng Git thực tế.

Changes có stage/unstage theo file, hunk và dòng text đủ điều kiện; commit và amend đã qua review cùng luồng native, gồm hook từ chối, giữ draft và bảo toàn staged/working edits. Chọn file giữ diff ổn định qua refresh thụ động; **Close diff** kết thúc phiên chọn. Amend phải bật rõ ràng, hỗ trợ chỉ đổi message; shallow repo chưa hỗ trợ amend. Sync có Push, Pull fast-forward và push/xóa tag remote; luồng HTTPS/SSH đã đối chiếu object/ref thực. Push hiển thị ACK có ngày riêng với local tracking: dùng Fetch để cập nhật counts và Local actions để đặt upstream. Đây là development checkpoint; Stash Save/Apply và nghiệm thu tích hợp vẫn phải hoàn tất trong0.3.0. Theo quyết định của người dùng, Pop/Drop chuyển sang0.4.0. Tiến độ và bằng chứng ở [task007](docs/tasks/007-implement-0.3.0.md), [plan](docs/plans/0.3.0.md) và [acceptance](docs/testing/0.3.0-acceptance.md). Chưa phát hành0.3.0 hoặc installer; enterprise GCM/OAuth/SSO được theo dõi riêng.

## Phạm vi sản phẩm

1. **Repository public/private:** clone qua HTTPS, SSH và local path khi người dùng có quyền; hỗ trợ Git host phổ biến và self-hosted thông qua Git, không khóa private repo theo gói ứng dụng.
2. **Quan sát:** graph với nhãn branch/remote/tag, commit details, diff, tìm kiếm lịch sử; Hide/Solo nhiều branch và nhớ bộ lọc theo repo.
3. **Làm việc hằng ngày:** fetch, quản lý/chuyển branch, tạo tag, stage/commit, pull/push, kết quả và lỗi rõ ràng.
4. **Tích hợp thay đổi:** merge, cherry-pick, rebase, giải quyết conflict, tiếp tục hoặc hủy thao tác đang dở, phục hồi sau khi mở lại app.
5. **Workspace nhiều repo:** cherry-pick giữa branch trong cùng repo/nhiều remote và giữa hai repository riêng.

"Mọi repo" là mục tiêu tương thích Git theo transport và quyền truy cập; không phải cam kết vượt qua SSO, VPN, chính sách server hoặc credentials thiếu quyền. Một repo có các remote tên riêng (ví dụ `origin`, `upstream`); hai repo trong workspace có thể có `origin` riêng.

## Stack

| Lớp | Công nghệ / quyết định |
| --- | --- |
| Desktop | Tauri 2; Windows trước, kiến trúc hỗ trợ mở rộng macOS/Linux |
| UI | React, TypeScript, Vite và component dùng chung |
| UI state | Zustand |
| Backend local | Rust, Tokio, typed IPC |
| Git | Git CLI cài trên máy; gọi bằng argument arrays |
| Xác thực | Git credential helpers/GCM cho HTTPS; OpenSSH và SSH agent cho SSH |
| Graph | Layout riêng, Canvas 2D và danh sách commit dùng TanStack Virtual |
| Diff | Monaco Editor chỉ đọc, tải khi cần với worker cục bộ; conflict workflow ở phiên bản sau |
| Dữ liệu app | SQLite cho workspace, registry, journal và trạng thái hiển thị theo repo |
| Kiểm thử hiện có | Rust tests trên repo tạm, Vitest; native Tauri/WebView2 qua tauri-driver và EdgeDriver |

Git là nguồn dữ liệu chính cho lịch sử và trạng thái thao tác. SQLite không thay thế Git. Không lưu credentials trong database/log của app. Dependency trực tiếp được pin exact trong manifests; dependency bắc cầu được khóa trong pnpm-lock.yaml và src-tauri/Cargo.lock.

## Phiên bản hiện tại

`VERSION` hiện là `0.3.0`: target đang triển khai. Checkpoint0.2.0 là `ce5da58`; target mới không phải tuyên bố đã hoàn tất/phát hành. Xem [quy tắc đánh version](docs/versioning.md).

Mở repo để vào History; chọn commit rồi chọn file để xem diff và các nhánh chứa commit. Với merge commit, chọn parent cần so sánh. Bấm hoặc chuột phải nhãn branch/tag trên graph để View commit, Hide/Show, Add/Remove from Solo hoặc mở thao tác local branch/tag. Nút **Local actions** có tạo/chuyển/đổi tên/xóa branch, đặt upstream và tạo/xóa tag. Solo có thể gồm nhiều branch; **References** mở danh sách đầy đủ, kể cả nhánh bị ẩn. Search tìm commit ngoài bộ lọc mà không đổi bộ lọc. View, lựa chọn và vị trí cuộn được nhớ theo repo; linked worktrees dùng chung preferences. **Changes** chứa staged/unstaged, diff và luồng stage/commit đang nghiệm thu. Cherry-pick nằm ở0.4.0.

Refresh tự động khi có notification, focus hoặc sau thao tác liên quan; fallback30 giây chỉ chạy khi cửa sổ hiện. Background refresh giữ nội dung; thao tác thủ công hoặc refresh chậm mới có feedback. Giới hạn history100.000 commit/session và các trường hợp không hỗ trợ được trả lỗi rõ ràng; xem [history contract](docs/architecture/history.md).

## Roadmap

| Version | Phạm vi | Trạng thái |
| --- | --- | --- |
| v0.1.0 | Workspace, mở/clone repo private, auth, task/error panel | Implementation candidate · under acceptance |
| v0.2.0 | Graph, sidebar, commit/diff, search, Hide/Solo, persistence; quiet refresh/watchers | Implementation candidate · native verified; gate status in evidence |
| v0.3.0 | Commit containment/remote evidence, fetch, branch/tag, stage/commit, stash, pull/push | Implementing · task007; not yet accepted |
| v0.4.0 | Merge, cherry-pick, rebase, conflict và recovery | Planned |
| v0.5.0 | Cherry-pick giữa repository trong workspace | Planned |
| v0.6.0 | LFS, submodule, worktree, signing UX, tương thích và hiệu năng | Planned |
| v1.0.0 | Kiểm chứng tích hợp, đóng gói và phát hành ổn định | Planned |

Phạm vi và điều kiện hoàn thành từng version: [docs/roadmap.md](docs/roadmap.md).

## Làm việc với Codex

Đọc [AGENTS.md](AGENTS.md) và [workflow](docs/development-workflow.md). Cấu hình nằm trong [.codex](.codex/README.md).

Orchestrator phân loại task trước khi sửa:

- **T0:** việc nhanh, ít rủi ro → tự làm và kiểm tra.
- **T1:** việc rõ ràng, phạm vi hẹp → checklist ngắn, tự làm hoặc một worker; review độc lập nếu thay đổi behavior.
- **T2:** task nhiều thành phần hoặc thay đổi workflow agent → planner → workers → reviewer.
- **T3:** auth, Git mutation/recovery, graph correctness hoặc concurrency → toàn bộ workflow với worker mạnh và kiểm chứng failure paths.

Reviewer trả comment bắt buộc → worker sửa → reviewer kiểm tra lại. Chỉ những subtask không tranh chấp quyền ghi và đã đủ dependency mới được chạy song song.

| Vai trò | Model mặc định | Reasoning |
| --- | --- | --- |
| Orchestrator | `gpt-6.1-sol` | `medium` |
| Planner | `gpt-6.1-sol` | `high` |
| Worker lặp lại / cơ học | `gpt-6-luna` | `medium` |
| Worker thông thường | `gpt-6.1-sol` | `medium` |
| Worker khó | `gpt-6.1-sol` | `high` |
| Reviewer độc lập | `gpt-6.1-sol` | `high` |

Ưu tiên model ít tốn nhất đủ khả năng, task nhỏ tự làm; planner dành cho T2/T3. Mặc định một worker, tối đa hai subagent đồng thời theo policy (host có thể thấp hơn), dù trần cấu hình hiện tại là 10. Hai worker song song cần phạm vi ghi riêng và lợi ích thời gian đủ bù chi phí context/tích hợp. Giữ review độc lập reasoning high; Astra/xhigh chỉ dùng khi có lý do năng lực/rủi ro được ghi lại, kể cả T3.

Task packet gọn, tái sử dụng nghiên cứu và agent cho vòng sửa khi phù hợp. Dự kiến số dispatch/vòng review và điểm đánh giá lại trong task record; không có budget mặc định. Ghi usage nếu runtime cung cấp, nếu không ghi chưa biết; không suy ra tiền hay % tiết kiệm từ quota thuê bao. Chi tiết escalation, giới hạn ngân sách nếu người dùng đặt sau này và kiểm chứng nằm trong workflow.

Worker đánh giá mức phù hợp và đề nghị đổi mức; orchestrator áp dụng bằng một lần dispatch mới. Agent không tự đổi model giữa một lượt đang chạy. Model availability và giới hạn đồng thời phụ thuộc host; các lựa chọn trên đã được đối chiếu với môi trường setup ngày 2026-10-24.

### Bắt đầu một task

Mở repository trong Codex, cho phép sử dụng cấu hình project qua cơ chế trust của client nếu cần, rồi bắt đầu/reload phiên để nhận cấu hình. Không cần sửa cấu hình Codex toàn máy. Lựa chọn model trong UI hoặc managed settings có thể ưu tiên hơn mặc định project.

Ví dụ yêu cầu:

> Triển khai phần nền tảng v0.1.0 theo docs/roadmap.md. Phân loại task, lập kế hoạch có acceptance criteria và chia worker theo phạm vi file. Chỉ bắt đầu các subtask đã đủ dependency; reviewer độc lập kiểm tra kết quả và yêu cầu sửa nếu cần.

Với T2/T3, orchestrator tạo task record từ [template](docs/tasks/TEMPLATE.md). Không bắt đầu toàn bộ roadmap khi người dùng chỉ giao một task.

### Kiểm tra setup hiện tại

Trong thư mục repository, dùng Python 3.11+ để kiểm tra cú pháp TOML:

```powershell
python -c "import pathlib,tomllib; fs=list(pathlib.Path('.codex').rglob('*.toml')); [tomllib.loads(p.read_text(encoding='utf-8')) for p in fs]; print(f'Parsed {len(fs)} TOML files')"
git diff --check
git status --short
```

Lệnh trên chỉ kiểm tra cấu hình Codex, không xác nhận ứng dụng đã build hoặc runtime đã nạp các role.

### Chạy ứng dụng và kiểm tra

Windows cần Node 24.19.0, pnpm 11.25.0, Rust 1.99.1, MSVC C++ build tools, Windows SDK và WebView2. Toolchain Rust được pin trong rust-toolchain.toml. Nếu dùng bộ công cụ cục bộ của repository, dot-source script trong mỗi PowerShell trước lệnh Rust/Tauri; script chỉ cấu hình phiên shell hiện tại:

```powershell
. .\scripts\dev-env.ps1
pnpm install --frozen-lockfile
pnpm check:version
pnpm typecheck
pnpm test
pnpm build
pnpm tauri dev
```

`pnpm dev` chạy riêng renderer tại http://127.0.0.1:1420; các API desktop cần chạy qua `pnpm tauri dev`. Renderer không có API shell/filesystem. Các lệnh kiểm tra Rust và build desktop:

```powershell
. .\scripts\dev-env.ps1
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri build --no-bundle
```

Feature `desktop` bật mặc định; `--no-default-features` cho phép test thư viện mà không cần WebView. Build desktop qua Tauri CLI tạo `src-tauri/target/release/opengit.exe` với frontend assets được nhúng; lệnh Cargo build trực tiếp có thể tạo binary trỏ tới dev URL. Chưa tạo bộ cài phát hành. Các fixture IPC dùng dữ liệu tổng hợp tại `fixtures/contracts/wire.json`. Kiểm tra renderer bằng Vitest không thay thế Windows desktop E2E hoặc live private-auth acceptance.

Kiểm tra native tự động dùng tauri-driver với EdgeDriver trùng phiên bản WebView2; hạ tầng nằm trong `tests/e2e/`. Feature `e2e` chỉ dành cho test: `pnpm tauri build --no-bundle --features e2e` biên dịch seam chooser đọc `OPENGIT_E2E_DIALOGS_JSON` là JSON array gồm absolute folder paths hoặc `null` (cancel). Mỗi phần tử được dùng một lần; không thêm IPC nhận path từ renderer. Bản production không biên dịch hoặc đọc biến test này và phải build lại không có feature `e2e` trước bàn giao. `OPENGIT_DATA_DIR` là override từ process environment cho profile dữ liệu được cấu hình; mặc định app dùng thư mục dữ liệu riêng do Tauri cung cấp.

## Tài liệu

- [Quy tắc cho agent](AGENTS.md)
- [Workflow, model routing và review](docs/development-workflow.md)
- [Roadmap và yêu cầu sản phẩm](docs/roadmap.md)
- [Kiến trúc dự kiến](docs/architecture.md)
- [Kế hoạch triển khai toàn sản phẩm](docs/plans/implementation-plan.md)
- [Packet triển khai 0.1.0](docs/plans/0.1.0.md)
- [Evidence acceptance 0.1.0](docs/testing/0.1.0-acceptance.md)
- [Quy tắc đánh version](docs/versioning.md)
- [Hướng dẫn cấu hình Codex](.codex/README.md)
- [Task records](docs/tasks/README.md)

License phát hành của OpenGit sẽ được chốt trước khi phân phối; repository hiện chưa có license file.
