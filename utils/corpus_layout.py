#!/usr/bin/env python

"""코퍼스 디렉토리 레이아웃 상수.

색인(utils/loader.py)과 불일치 검사(backend/book_manager.py)가 같은 목록을 써야 해서
양쪽이 함께 참조한다. 두 모듈이 서로를 import하면 순환이 나므로 아무것도 import하지
않는 이 모듈에 둔다.
"""

# 책이 아니라 OCR·EPUB 빌드 파이프라인의 중간 산출물이 쌓이는 디렉토리.
# 한쪽에서만 빼면 이미 색인된 문서가 전부 "파일 없는 고아"로 잡히고, 재적재를 누르면
# 디스크에 멀쩡히 있는 파일의 문서를 지운다(실측 93,602건).
IGNORED_DIR_NAMES = frozenset({"page_images", "tesseract_text", "source_chapters", "OEBPS", "META-INF"})
