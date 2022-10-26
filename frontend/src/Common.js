import {sprintf} from 'sprintf-js';
import {str} from 'crc-32';

export function getUrlPrefix() {
  return process.env.REACT_APP_API_URL_PREFIX;
}

export function handleFetchErrors(response) {
  if (!response.ok) {
    throw Error(response.statusText);
  }
  return response;
}

const getRandomColorWithRange = (key, base, range) => {
  const i = str(key) % (16 * 16 * 16);
  const i1 = ((i >> 0) % range) + base;
  const i2 = ((i >> 4) % range) + base;
  const i3 = ((i >> 8) % range) + base;
  return sprintf("#%01x%01x%01x", i1, i2, i3);
}

const getRandomDarkestColor = (str) => {
  return getRandomColorWithRange(str, 4, 4);
};

export const getRandomDarkColor = (str) => {
  return getRandomColorWithRange(str, 6, 4);
};

const getRandomMediumColor = (str) => {
  return getRandomColorWithRange(str, 8, 4);
};

const getRandomLightColor = (str) => {
  return getRandomColorWithRange(str, 10, 4);
};

const getRandomLightestColor = (str) => {
  return getRandomColorWithRange(str, 12, 4);
};

const apiReq = (url, method, type, resolve, reject, final) => {
  try {
    fetch(getUrlPrefix() + url, {method: method})
      .then(handleFetchErrors)
      .then((response) => {
        let promise;
        if (type === 'JSON') {
          promise = response.json();
        } else if (type === 'TEXT') {
          promise = response.text();
        } else {
          promise = response.blob();
        }
        return promise;
      })
      .then((data) => {
        if (type === 'JSON') {
          if (data['status'] === 'success') {
            resolve(data['result']);
          } else {
            reject(data['error']);
          }
        } else {
          resolve(data);
        }
      })
      .catch((error) => {
        if (reject) {
          reject(error);
        }
      })
      .finally(() => {
        if (final) {
          final();
        }
      });
  } catch (error) {
    if (reject) {
      reject(error);
    }
  }
};

export const jsonGetReq = (url, resolve, reject, final) => apiReq(url, 'GET', 'JSON', resolve, reject, final);
export const jsonPutReq = (url, resolve, reject, final) => apiReq(url, 'PUT', 'JSON', resolve, reject, final);
export const jsonDeleteReq = (url, resolve, reject, final) => apiReq(url, 'DELETE', 'JSON', resolve, reject, final);
export const textGetReq = (url, resolve, reject, final) => apiReq(url, 'GET', 'TEXT', resolve, reject, final);
export const blobGetReq = (url, resolve, reject, final) => apiReq(url, 'GET', 'BLOB', resolve, reject, final);
