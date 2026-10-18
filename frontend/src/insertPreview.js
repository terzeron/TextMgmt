import { getApiUrlPrefix } from "./Common";
import { describeHttpError } from "./httpError";

// 서버가 받는 이미지 한도(backend MAX_IMAGE_BYTES)와 같다. 넘으면 올리기 전에 알린다.
export const MAX_UPLOAD_BYTES = 15 * 1024 * 1024;

// PDF 쪽 삽입과 EPUB 표지가 함께 쓰는 미리보기 요청이다.
// source 는 공개 이미지 URL(문자열)이거나 이 기기에서 고른 파일(File)이다.
// 서버가 정규화한 JPEG Blob 을 돌려주고, 실패하면 사람이 읽을 사유를 담은 Error 를 던진다.
export async function requestInsertPreview(apiPrefix, bookId, source) {
  const isFile = typeof source !== "string";
  if (isFile && source.size > MAX_UPLOAD_BYTES) {
    throw new Error("이미지는 15MB 이하여야 합니다");
  }
  const base = `${getApiUrlPrefix()}${apiPrefix}/pdf-pages/${bookId}`;
  let response;
  if (isFile) {
    const form = new FormData();
    form.append("file", source);
    response = await fetch(`${base}/insert-preview-upload`, {
      method: "POST",
      credentials: "include",
      body: form,
    });
  } else {
    response = await fetch(`${base}/insert-preview`, {
      method: "POST",
      credentials: "include",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url: source }),
    });
  }
  if (!response.ok) {
    // 400 은 서버가 사람이 읽을 사유를 본문에 담아 보낸 것이다.
    if (response.status === 400) {
      const reason = (await response.text()).trim();
      if (reason) throw new Error(reason);
    }
    throw new Error(await describeHttpError(response));
  }
  return response.blob();
}

// 저장 요청에 싣기 위해 Blob 을 base64 문자열로 바꾼다(data URL 의 접두어는 뺀다).
export function blobToBase64(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1]);
    reader.onerror = () => reject(new Error("이미지를 읽지 못했습니다"));
    reader.readAsDataURL(blob);
  });
}
