export function getUrlPrefix() {
  return process.env.REACT_APP_API_URL_PREFIX;
}

export function handleFetchErrors(response) {
  if (!response.ok) {
    throw Error(response.statusText);
  }
  return response;
}
