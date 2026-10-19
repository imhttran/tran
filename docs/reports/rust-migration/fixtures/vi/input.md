# Bộ dữ liệu gốc — truy xuất tiếng Việt

Tài liệu này là đầu vào cố định cho fixture `vi-retrieval` của RUST-001. Nó được
chọn để kiểm tra cấu hình FTS tiếng Việt, phân đoạn theo tiêu đề Markdown và việc
giữ nguyên ngôn ngữ (`CORPUS_LANGUAGE`) qua các bước xử lý.

## Cài đặt

Trình cài đặt kiểm tra mã băm trước khi ghi tệp. Cấu hình được đọc từ biến môi trường.

## Truy xuất

Truy xuất kết hợp tìm kiếm vector với tìm kiếm toàn văn. Hai danh sách được hợp nhất
bằng reciprocal rank fusion, sau đó loại bỏ trùng lặp và mở rộng theo từng phần.

## Trích dẫn

Câu trả lời trích dẫn nguồn theo dạng [nguồn - phần]. Trình kiểm tra trích dẫn phân
tích, xác thực và sửa chữa trích dẫn trước khi hiển thị.
