<!--
Fixture: page-provenance
Source form: text-layer PDF (this Markdown records the page text; the capture step
converts it to a one-page-per-section PDF and ingests it with the RAG-007 PDF loader).
reference_sha: d72fb398e8959eea83ed24be03f6df8cdeca86a2
-->

## page 1

Hướng dẫn cài đặt. Trình cài đặt kiểm tra mã băm trước khi ghi tệp. Cấu hình được
đọc từ biến môi trường.

## page 2

Truy xuất kết hợp tìm kiếm vector với tìm kiếm toàn văn. Hai danh sách được hợp nhất
bằng reciprocal rank fusion, sau đó loại bỏ trùng lặp và mở rộng theo từng phần.

## page 3

Câu trả lời trích dẫn nguồn theo dạng [nguồn - phần]. Số trang 1-based chảy từ
loader qua chunking, lưu trữ, truy xuất và hiển thị trích dẫn.
