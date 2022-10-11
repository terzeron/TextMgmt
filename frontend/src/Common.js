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

