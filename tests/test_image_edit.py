"""crop_pil: PDF 쪽과 EPUB 이미지가 함께 쓰는 크롭 기하."""

import pytest
from PIL import Image

from backend.image_edit import ImageCropError, crop_pil


def test_axis_aligned_crop_uses_fractions_of_the_image():
    out = crop_pil(Image.new("RGB", (200, 100)), (0.25, 0.1, 0.75, 0.9, 0))
    assert out.size == (100, 80)


def test_tilted_crop_straightens_the_region():
    source = Image.new("RGB", (200, 200), "white")
    out = crop_pil(source, (0.3, 0.3, 0.7, 0.7, 10))
    assert out.size == (80, 80)


def test_one_bit_image_is_rotated_as_grey():
    out = crop_pil(Image.new("1", (200, 200), 1), (0.3, 0.3, 0.7, 0.7, 5))
    assert out.mode == "L"


@pytest.mark.parametrize("rect,reason", [
    ((0.5, 0.1, 0.4, 0.9, 0), "밖이거나 비어"),
    ((0, 0, 1.2, 1, 0), "밖이거나 비어"),
    ((0, 0, float("nan"), 1, 0), "밖이거나 비어"),
    ((0, 0, 0.01, 1, 0), "너무 작음"),
    ((0, 0, 1, 1, 30), "쪽 밖으로 나감"),
])
def test_invalid_regions_raise_with_a_reason(rect, reason):
    with pytest.raises(ImageCropError, match=reason):
        crop_pil(Image.new("RGB", (200, 200)), rect)
