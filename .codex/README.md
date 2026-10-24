# Cấu hình Codex của OpenGit

Đây là cấu hình theo repository, không sửa cấu hình toàn cục hay quyền của máy. Quy trình và quyền sở hữu công việc nằm trong [AGENTS.md](../AGENTS.md) và [development-workflow.md](../docs/development-workflow.md).

## Mô hình và vai trò

`config.toml` đặt mặc định cho agent chính là `gpt-6.1-sol` / `medium`. Agent chính điều phối, trừ khi nhiệm vụ hiện tại giao rõ một vai trò chuyên trách. File `agents/orchestrator.toml` là định nghĩa có thể dùng lại khi được giao vai trò đó; nó không tự tạo thêm một agent chính.

| Vai trò | Model | Reasoning | Phạm vi |
| --- | --- | --- | --- |
| `orchestrator` | `gpt-6.1-sol` | `medium` | Phân loại, phân công, tích hợp, nghiệm thu |
| `planner` | `gpt-6.1-sol` | `high` | T2/T3: khảo sát và trả kế hoạch; không sửa repository |
| `worker` | `gpt-6-luna` | `medium` | Việc lặp lại, mẫu và tiêu chí rõ |
| `worker_standard` | `gpt-6.1-sol` | `medium` | Tính năng và gỡ lỗi thông thường |
| `worker_complex` | `gpt-6.1-sol` | `high` | Thuật toán mơ hồ, thao tác rủi ro, lỗi khó |
| `reviewer` | `gpt-6.1-sol` | `high` | Review độc lập; không sửa repository |

Agent không chọn vai trò dùng mặc định `gpt-6.1-sol` / `medium` trong `[agents]`. Trần cấu hình hiện tại là 10; policy vận hành tối đa hai subagent đồng thời, thường một worker, và host có thể thấp hơn. Hai worker song song chỉ khi task đủ lớn, quyền ghi riêng và lợi ích thời gian được ghi lại đủ bù context/tích hợp. Đây là chỉ dẫn, không phải cơ chế áp ngân sách tự động. Hạn chế ghi của planner/reviewer là chỉ dẫn vai trò, không phải sandbox riêng. Các vai trò kế thừa quyền, provider và MCP của phiên cha.

## Kích hoạt và phân công

Mở repository như project tin cậy trong Codex rồi tạo hoặc tải lại phiên để dùng cấu hình này. Cấu hình project chỉ được nạp khi project được tin cậy; tài liệu này không tự cấp trust. Lựa chọn model/reasoning trong UI hoặc lúc khởi chạy có thể ghi đè mặc định của phiên. Không suy ra rằng thay file đã đổi model của agent đang chạy; kiểm tra thiết lập thực tế và ghi lại sai khác trong task record.

Codex hiện hỗ trợ tự phát hiện file TOML độc lập trong `.codex/agents/`; mỗi file có `name`, `description`, `developer_instructions` và mặc định model/reasoning. Dùng tên vai trò nếu công cụ phân công hỗ trợ. `name` là định danh, tên file được giữ trùng để dễ kiểm tra.

Model/effort được đặt rõ trong named role có thể ưu tiên hơn override lúc spawn. Muốn escalation ngoại lệ, dùng fallback phân công rõ model/effort dưới đây mà không chọn named role cố định; nếu công cụ không hỗ trợ, báo giới hạn năng lực.

Khi công cụ không có bộ chọn named role, orchestrator đọc TOML vai trò, đưa đầy đủ `developer_instructions` cùng task packet tự chứa vào lệnh phân công, rồi truyền rõ model và reasoning được hỗ trợ. Với API collaboration của phiên này, dùng `fork_turns="none"` khi truyền override; full-history fork không nhận override model/effort. Cách này áp dụng nội dung file bằng prompt, không chứng minh rằng runtime đã tự nạp named role. Task packet phải nêu yêu cầu, tiêu chí nghiệm thu, input, dependency, file được ghi, kiểm tra và điều kiện hoàn thành.

Ưu tiên model ít tốn nhất đủ khả năng, task nhanh tự làm và planner cho T2/T3. Sol/high xử lý mặc định việc critical đã rõ; T3 không tự động chọn Astra/xhigh. Astra/xhigh cần lý do cụ thể được ghi lại: thiếu năng lực, unknown khó hoặc lỗi lặp lại có bằng chứng. Có thể escalation ngay khi rủi ro nghiêm trọng chưa giải quyết; không bắt thử model yếu trước. Giữ review độc lập reasoning high và kiểm chứng bắt buộc.

Task packet gọn, dùng lại nghiên cứu/evidence, gộp việc cơ học liên quan, dùng lại worker/reviewer cho vòng sửa khi phù hợp. Review delta và regression bị ảnh hưởng vẫn phải bao phủ toàn bộ acceptance. Dừng cùng cách làm sau hai lần thất bại và đánh giá lại. Task record ghi số dispatch/vòng review dự kiến và checkpoint mềm; không có budget mặc định, phải tôn trọng cap người dùng đặt sau này và báo đúng việc chưa xong. Usage không có dữ liệu thì ghi chưa biết; không công bố tiền/% tiết kiệm nếu thiếu usage đo được, cách tính bill và giá hiện hành. Quota thuê bao khác API bill.

Worker chỉ đề xuất escalation có bằng chứng. Orchestrator phân công agent thay thế để áp dụng model/effort khác; worker không tự đổi model. Nếu model không khả dụng, dùng fallback trong [development-workflow.md](../docs/development-workflow.md), ghi đúng runtime thực tế và giữ reviewer độc lập.

Không đặt `sandbox_mode`, `approval_policy`, provider hay MCP riêng trong file vai trò. Cấu hình này không hạ quyền bảo vệ và không dùng profile cũ để giả lập vai trò. Không tự thêm feature flag nếu tài liệu hiện tại không yêu cầu.

## Kiểm tra

Từ root repository, dùng Python 3.11+ có sẵn trong môi trường để parse toàn bộ TOML, ví dụ:

```powershell
python -c "import pathlib,tomllib; files=list(pathlib.Path('.codex').rglob('*.toml')); [tomllib.loads(p.read_text(encoding='utf-8')) for p in files]; print(f'Parsed {len(files)} TOML files')"
```

Lệnh này chỉ kiểm tra cú pháp; đối chiếu thêm model matrix và chỉ dẫn vai trò với `AGENTS.md`/workflow. Bootstrap chưa có app, manifest hay bộ build/test. Việc parse thành công không chứng minh cấu hình đã được runtime nạp hoặc từng model khả dụng trên mọi tài khoản.

Schema và cách kế thừa được đối chiếu với [OpenAI Docs: Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents) và [Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference). Kiểm tra lại tài liệu chính thức khi nâng Codex.

Tham khảo chọn model và yếu tố usage: [Models](https://learn.chatgpt.com/docs/models), [Pricing](https://learn.chatgpt.com/docs/pricing).
