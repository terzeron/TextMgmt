"""PDF 쪽 크롭과 EPUB 이미지 크롭이 함께 쓰는 PIL 크롭 기하."""

import math
from typing import Any

from PIL import Image

Rect = tuple[float, float, float, float, float]  # x0, y0, x1, y1 (이미지 크기 대비 0~1, 원점은 좌상단), angle (도, 시계 방향, 박스 중심 기준)

MIN_CROP_PIXELS = 8


class ImageCropError(ValueError):
    """크롭할 수 없는 사유(호출부가 앞에 위치 정보를 붙인다)."""


def crop_pil(source: Image.Image, rect: Rect, *, min_pixels: int = MIN_CROP_PIXELS) -> Image.Image:
    """source 에서 rect(x0, y0, x1, y1, angle) 영역만 잘라 새 이미지로 돌려준다.

    angle 이 0이 아니면 기울어진 영역을 곧게 펴서 잘라내며(재샘플링), 네 꼭짓점이 이미지 안에 있어야 한다.
    영역이 이미지 밖이거나 비어 있거나 한 변이 min_pixels 미만이면 ImageCropError 를 던진다.
    """
    x0, y0, x1, y1, angle = rect
    if not all(math.isfinite(v) for v in rect) or not (0 <= x0 < x1 <= 1 and 0 <= y0 < y1 <= 1) or abs(angle) > 360:
        raise ImageCropError("크롭 영역이 쪽 밖이거나 비어 있음")
    width, height = source.size
    if abs(angle) < 0.01:
        left, top, right, bottom = round(x0 * width), round(y0 * height), round(x1 * width), round(y1 * height)
        out_w, out_h = right - left, bottom - top
    else:
        out_w, out_h = round((x1 - x0) * width), round((y1 - y0) * height)
    if out_w < min_pixels or out_h < min_pixels:
        raise ImageCropError("크롭 영역이 너무 작음")
    if abs(angle) < 0.01:
        return source.crop((left, top, right, bottom))
    theta = math.radians(angle)
    cos_t, sin_t = math.cos(theta), math.sin(theta)
    center_x, center_y = (x0 + x1) / 2 * width, (y0 + y1) / 2 * height
    half_w, half_h = out_w / 2, out_h / 2
    for sign_x, sign_y in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        corner_x = center_x + cos_t * sign_x * half_w - sin_t * sign_y * half_h
        corner_y = center_y + sin_t * sign_x * half_w + cos_t * sign_y * half_h
        if not (-1.5 <= corner_x <= width + 1.5 and -1.5 <= corner_y <= height + 1.5):
            raise ImageCropError("기울어진 크롭 영역이 쪽 밖으로 나감")
    # 출력 픽셀 (x, y) → 원본 좌표: 박스 중심을 기준으로 angle 만큼 돌린 위치
    coefficients: Any = (cos_t, -sin_t, center_x - cos_t * half_w + sin_t * half_h, sin_t, cos_t, center_y - sin_t * half_w - cos_t * half_h)
    # 1비트 이미지는 보간할 수 없으므로 8비트 회색으로 올려서 돌린다
    rotating = source.convert("L") if source.mode == "1" else source
    fill: float | tuple[float, ...] = 255 if rotating.mode == "L" else (255, 255, 255)
    return rotating.transform((out_w, out_h), Image.Transform.AFFINE, coefficients, resample=Image.Resampling.BICUBIC, fillcolor=fill)
