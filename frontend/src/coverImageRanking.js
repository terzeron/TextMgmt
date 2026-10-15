export function rankCoverImages(candidates, images) {
  const available = candidates.map(({ candidate_id, width, height }) => ({
    id: candidate_id, area: (images[candidate_id]?.width || width || 0) * (images[candidate_id]?.height || height || 0),
  })).filter(({ area }) => area > 0).sort((a, b) => b.area - a.area);
  const ranks = {};
  let previousArea;
  let rank = 0;
  available.forEach(({ id, area }, index) => {
    // 같은 해상도의 후보는 같은 색으로 표시한다.
    if (area !== previousArea) rank = index;
    previousArea = area;
    const ratio = rank / Math.max(1, available.length - 1);
    const red = Math.round(255 * Math.min(1, 2 * ratio)).toString(16).padStart(2, "0");
    const green = Math.round(255 * Math.min(1, 2 * (1 - ratio))).toString(16).padStart(2, "0");
    ranks[id] = { rank: rank + 1, color: `#${red}${green}00`, count: available.length };
  });
  return ranks;
}

export function formatCoverBytes(bytes) {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`;
}
