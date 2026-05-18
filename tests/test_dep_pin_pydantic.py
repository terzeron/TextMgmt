#!/usr/bin/env python
"""pydantic dependency pinning.

backend 사용처:
- backend/main.py:23 from pydantic import BaseModel
  여러 요청/응답 모델 정의:
    - BookModel (int/str/float/기본값)
    - CategoryRenameModel, CategoryDeleteModel
    - CategoryKeywordsModel (keywords: list[str])
    - CategoryMappingsModel (mappings: dict[str, list[str]])
    - HiddenCategoryModel

박제 API:
- pydantic.BaseModel 서브클래싱
- 기본값 지정
- list[str], dict[str, list[str]] 타입 어노테이션 지원
- 잘못된 타입 입력 시 ValidationError
- model_dump() / dict()
"""

import unittest


class TestBaseModelImport(unittest.TestCase):
    def test_import_base_model_and_validation_error(self):
        from pydantic import BaseModel, ValidationError

        self.assertTrue(callable(BaseModel))
        self.assertTrue(issubclass(ValidationError, Exception))


class TestBookModelShape(unittest.TestCase):
    """main.py:281 BookModel 패턴"""

    def _make_model(self):
        from pydantic import BaseModel

        class BookModel(BaseModel):
            book_id: int
            category: str
            title: str
            author: str
            file_path: str
            file_type: str
            file_size: int
            line_count: int = 0
            page_count: int = 0
            isbn: str = ""
            updated_time: str
            score: float = 0.0

        return BookModel

    def test_construct_with_required_fields(self):
        BookModel = self._make_model()
        m = BookModel(book_id=1, category="fiction", title="t", author="a", file_path="/x", file_type="txt", file_size=10, updated_time="2024-01-01")
        self.assertEqual(m.book_id, 1)
        self.assertEqual(m.line_count, 0)
        self.assertEqual(m.page_count, 0)
        self.assertEqual(m.isbn, "")
        self.assertEqual(m.score, 0.0)

    def test_missing_required_raises_validation_error(self):
        from pydantic import ValidationError

        BookModel = self._make_model()
        with self.assertRaises(ValidationError):
            BookModel(book_id=1)


class TestSimpleStringFields(unittest.TestCase):
    """main.py: CategoryRenameModel, CategoryDeleteModel"""

    def test_category_rename_model(self):
        from pydantic import BaseModel

        class CategoryRenameModel(BaseModel):
            old_category: str
            new_category: str

        m = CategoryRenameModel(old_category="A", new_category="B")
        self.assertEqual(m.old_category, "A")
        self.assertEqual(m.new_category, "B")

    def test_category_delete_model(self):
        from pydantic import BaseModel

        class CategoryDeleteModel(BaseModel):
            category: str

        m = CategoryDeleteModel(category="X")
        self.assertEqual(m.category, "X")


class TestListField(unittest.TestCase):
    """main.py:768 CategoryKeywordsModel"""

    def test_list_str_field(self):
        from pydantic import BaseModel

        class CategoryKeywordsModel(BaseModel):
            keywords: list[str]

        m = CategoryKeywordsModel(keywords=["a", "b", "c"])
        self.assertEqual(m.keywords, ["a", "b", "c"])

    def test_list_field_type_coercion(self):
        from pydantic import BaseModel, ValidationError

        class M(BaseModel):
            xs: list[str]

        with self.assertRaises(ValidationError):
            M(xs="not-a-list")


class TestDictField(unittest.TestCase):
    """main.py:772 CategoryMappingsModel — dict[str, list[str]]"""

    def test_dict_of_list(self):
        from pydantic import BaseModel

        class CategoryMappingsModel(BaseModel):
            mappings: dict[str, list[str]]

        m = CategoryMappingsModel(mappings={"cat1": ["k1", "k2"], "cat2": ["k3"]})
        self.assertEqual(m.mappings["cat1"], ["k1", "k2"])
        self.assertEqual(m.mappings["cat2"], ["k3"])


class TestModelDumpAndDict(unittest.TestCase):
    """main.py에서 응답 시 .model_dump() 사용 (혹은 FastAPI가 직접 직렬화)"""

    def test_model_dump_returns_dict(self):
        from pydantic import BaseModel

        class M(BaseModel):
            a: int
            b: str = "x"

        m = M(a=1)
        d = m.model_dump()
        self.assertIsInstance(d, dict)
        self.assertEqual(d, {"a": 1, "b": "x"})


class TestValidationError(unittest.TestCase):
    def test_int_field_rejects_non_numeric_string(self):
        from pydantic import BaseModel, ValidationError

        class M(BaseModel):
            v: int

        with self.assertRaises(ValidationError):
            M(v="not_a_number")

    def test_float_field_accepts_int(self):
        from pydantic import BaseModel

        class M(BaseModel):
            v: float = 0.0

        m = M(v=1)
        self.assertEqual(m.v, 1.0)


if __name__ == "__main__":
    unittest.main()
