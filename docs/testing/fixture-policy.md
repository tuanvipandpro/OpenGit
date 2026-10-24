# Foundation failure fixtures

OG-010-08 cung cấp `src-tauri/tests/support/{mod,repos,faults}.rs`, dùng trong target `foundation_failures`. Đây là harness test, không phải production IPC hay fault backdoor.

## Phạm vi và ownership

- Mỗi `OwnedFixture` tạo thư mục tạm tên ngẫu nhiên, marker UUID và ghi nhớ canonical root cùng OS directory identity. Working/bare repositories, SQLite, scripts và sentinels đều nằm trong root đó. Git identity chỉ truyền bằng `-c` cho command fixture.
- Cleanup chỉ nhận đúng root đã cấp quyền; từ chối checkout OpenGit, parent, root khác, marker thay đổi, identity thay đổi và reparse/symlink trong cây. Kiểm tra toàn cây có giới hạn trước `remove_dir_all`; không chuyển danh sách đường dẫn sang shell để xóa. Nếu guard hoặc I/O thất bại, giữ phần fixture còn lại và trả lỗi; `Drop` không xóa bỏ qua guard.
- Harness không giả định cleanup là nguyên tử: lỗi giữa chừng có thể để lại một phần root. Test cleanup failure chỉ phục hồi marker đã lưu của chính root đó sau khi đóng file handle, rồi kiểm guard và retry. Dữ liệu ngoài root phải giữ nguyên.
- Chỉ fixture code đã tin cậy được ghi vào root. Guard không phải bảo đảm chống actor cùng user cố tình đổi đường dẫn đồng thời giữa kiểm tra và xóa; không dùng harness để xóa thư mục do người dùng chọn.

## Fault seams và evidence

- `GitFixtures` tạo working repository có một commit rỗng và bare repository bằng Git thật; không dùng checkout phát triển hoặc production remote.
- `ReadOnlyFile` giữ/khôi phục permissions gốc bằng RAII. SQLite migration phải thất bại thật trên file chỉ đọc và giữ nguyên bytes. Trigger SQLite gây journal write failure thật; kiểm tra snapshot/fence không đổi và không có event.
- Windows process faults dùng absolute PowerShell executable qua production `GitRunner`, với script cố định bên trong fixture: exit lỗi với synthetic canary, sleep để timeout, output vượt cap. Không mở shell API cho renderer hoặc tắt TLS/SSH verification.
- `OPENGIT_SYNTHETIC_CANARY_08_NOT_A_CREDENTIAL` chỉ là chuỗi tổng hợp. Kiểm tra nó không xuất hiện trong structured error hoặc SQLite; source/destination sentinels phải giữ nguyên sau fault. Không đưa credentials thật vào fixture, snapshot hay log.
- Negative cleanup checks gồm checkout/parent/root khác, marker sai và reparse child; Windows locked file gây cleanup I/O failure thật, sau đó retry trong root được xác minh.

Các test 03–06 vẫn tự chứa để giữ dependency DAG một chiều. Test restart, cancellation, contention và event sequence/fence nằm trong `operation_lifecycle`; harness08 không nhân bản chúng. Disk-full, process crash/publish và destination races cần seam của clone packet10, chưa được harness này tuyên bố kiểm chứng.

Transport HTTPS/SSH thật do orchestrator sở hữu ở `tests/e2e/support/live_remotes.py`; kiểm tra auth/clone tích hợp thuộc các packet09–12. Fixture local/process và lỗi authentication tổng hợp không chứng minh private authentication, server permission, CA, host-key hoặc SSO.

Runtime clone cleanup có yêu cầu mạnh hơn harness: không dùng `remove_dir_all` theo pathname. Nó chỉ disposition đúng marker/đối tượng rỗng qua handle đã xác thực; cây không rỗng hoặc ownership không chứng minh được phải giữ lại cùng cảnh báo journal bền vững. `clone_transaction` kiểm tracked Git checkout/publication, cancel/publish races, lỗi SQLite sau publish, restart không chạy lại network và repeated recovery giữ cả cây đã chuyển lẫn replacement sentinel. Unit staging chèn ordinary-directory/junction substitution đúng boundary; unit auth chèn cancellation sau khi nhận secret nhưng trước release. Các hook chỉ tồn tại trong `cfg(test)`, không có fault IPC ở production.

## Kiểm tra

Trong PowerShell ở repository:

```powershell
. .\scripts\dev-env.ps1
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --offline --test foundation_failures
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --offline
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --offline --all-targets -- -D warnings
```

Kết quả thực tế do handoff/task record ghi; danh sách lệnh không tự xác nhận PASS. Harness không thêm dependency, live credential hoặc production test command.
