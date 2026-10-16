// 상태 코드만 노출하면 원인을 알 수 없으므로 본문을 함께 보여준다.
export async function describeHttpError(response) {
  let detail = "";
  try {
    detail = (await response.text()).trim();
  } catch {
    // 본문을 읽지 못하면 상태 코드만으로 보고한다.
  }
  return detail
    ? `HTTP ${response.status} - ${detail.slice(0, 200)}`
    : `HTTP ${response.status}`;
}
