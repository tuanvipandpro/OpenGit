# Environment/readiness matrix — 0.1.0

> Historical readiness snapshot for packet01. Later build, implementation and native acceptance results supersede the NOT_RUN/BLOCKED entries below: see [current evidence](0.1.0-acceptance.md). This table is preserved as initial evidence, not the current release verdict.

OG-010-01, 2026-10-24, Windows hiện tại. Evidence tái sử dụng từ orchestrator/task 003; worker contract không chạy lại provisioning. `DETECTED` chỉ xác nhận version/sự hiện diện, không chứng minh build hay GUI auth. Không có budget mặc định; runtime usage **unknown**; không có phép đo billing/cost savings.

| Thành phần / gate | Evidence được cung cấp | Trạng thái / kiểm tra còn cần |
| --- | --- | --- |
| Git | `git --version`: `2.56.0.windows.2` | DETECTED; runner/porcelain/transport fixtures chưa chạy |
| Node | `node --version`: `24.19.0` | DETECTED; scaffold compatibility/build chưa chạy |
| pnpm | orchestrator chạy version: `11.25.0` | DETECTED; install/lockfile reproducibility chưa chạy |
| GCM | `git credential-manager --version`: `2.9.1` | DETECTED; GUI/private/SSO chưa chứng minh |
| OpenSSH | `ssh -V`: `9.5p2` | DETECTED; GUI key/passphrase/host-key chưa chứng minh |
| WebView2 | registry runtime `155.0.4283.62` | DETECTED; Tauri desktop launch chưa chạy |
| Rust | `1.99.1` tại `.tools/{cargo,rustup}`, named toolchain kèm rustfmt/clippy | PASS compiler/linker smoke: rustc build và chạy executable |
| MSVC/Windows SDK | portable MSVC `14.44.35207`, SDK `10.0.26100.7705`; gói Microsoft được extractor kiểm SHA256 | PASS: cl build/link chương trình dùng windows.h/GetCurrentProcessId, executable chạy exit 0 |
| Dependencies/SQLite | Tauri 2/React/TS/Vite/Tokio; đề xuất rusqlite bundled | PENDING; owner 02 chốt exact pins/lockfiles sau compatibility |
| Contract/design | [IPC v1](../architecture/ipc-contract.md), [foundation](../architecture/foundation.md) | READY_FOR_REVIEW về thiết kế; typed fixture/round-trip do 02 |
| Build readiness | `. scripts/dev-env.ps1`; cl/sdk smoke và rustc linker smoke thành công | READY cho scaffold; Tauri app build/launch vẫn là gate 02 chưa chạy |
| Private HTTPS/SSH fixture | chưa cung cấp môi trường/truy cập được cấp phép | BLOCKED live acceptance; không yêu cầu ghi secrets vào tài liệu |

Root đã chạy `cl.exe /nologo .tools/sdk-smoke.c /Fo.tools/sdk-smoke.obj /Fe.tools/sdk-smoke.exe` (windows.h/GetCurrentProcessId), `.tools/sdk-smoke.exe` exit 0; `rustc .tools/smoke.rs -o .tools/rust-smoke.exe` và executable in thông báo smoke. Toolchain chỉ thêm vào shell hiện tại qua `scripts/dev-env.ps1`, không sửa PATH toàn máy. MSI administrative extraction cần sandbox elevation và đã hoàn tất; cleanup extractor được guard trong `.tools/msvc`/`.tools/downloads`. Owner 02 tiếp tục ghi commands thật/build/launch/reproducibility; không suy ra app tests đã đạt từ compiler smoke.

| Required validation gate | Owner | Trạng thái hiện tại |
| --- | --- | --- |
| Windows toolchain/build/launch từ lockfiles | root/02 | NOT_RUN; chờ provisioning và scaffold |
| Typed round-trip, malformed request, missing Git, parser/path redaction | 02/03/05/08 | NOT_RUN |
| Workspace alias/bare/linked/nested + migration/corrupt DB | 04/05/08 | NOT_RUN |
| Lifecycle/cancel/sequence/generation/DB journal/restart | 06/08 | NOT_RUN |
| GUI HTTPS helper/browser + SSH agent/passphrase/host key | 09 | NOT_RUN |
| Public HTTPS/SSH clone từ GUI | 09/10/12 | NOT_RUN; cần authorized test remotes |
| Private HTTPS/SSH; denied/expired/SSO/browser cancel/proxy/CA/host mismatch | 09/12 | BLOCKED môi trường/truy cập live chưa có; simulated coverage ghi riêng |
| Local normal/bare clone, publish race/disk/permission/late cancel/crash | 10/12 | NOT_RUN; chỉ disposable repositories |
| UI stale event/poll/reopen/focus + secret canary scan UI/log/SQLite | 07/11/12 | NOT_RUN |
| Independent review toàn acceptance A1/A6/F1–F5 | reviewer/root | NOT_RUN; design READY_FOR_REVIEW không phải approval |

Ranh giới kết luận: OG-010-01 chỉ cung cấp contract và tách blockers. F1/A1 chưa được nghiệm thu bằng tài liệu. Không release/tag, không tuyên bố compatibility của provider hay private auth khi required gate còn BLOCKED/NOT_RUN.
