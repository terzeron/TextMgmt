import {sprintf} from 'sprintf-js';
import {str} from 'crc-32';
import {DateTime} from 'luxon';

export function getApiUrlPrefix() {
    if (!import.meta.env.VITE_API_URL_PREFIX) {
        console.log("The environment variable VITE_API_URL_PREFIX is not set.");
    }
    return import.meta.env.VITE_API_URL_PREFIX;
}

export function handleFetchErrors(response) {
    if (!response.ok) {
        throw Error(response.statusText);
    }
    return response;
}

const getRandomColorWithRange = (key, base, range) => {
    let i = str(key + "_saltstring") % (256 * 256 * 256);
    if (i < 0) {
        i = i * -1;
    }
    let i1 = ((i >> 0) % range) + base;
    let i2 = ((i >> 8) % range) + base;
    let i3 = ((i >> 16) % range) + base;

    if (i1 > i2 && i1 > i3) {
        if (i2 > i3) {
            i2 = i2 * 0.6;
            i3 = i3 * 0.3;
        } else {
            i2 = i2 * 0.3;
            i3 = i3 * 0.6;
        }
    }
    if (i2 > i1 && i2 > i3) {
        if (i1 > i3) {
            i3 = i3 * 0.1;
            i1 = i1 * 0.7;
        } else {
            i3 = i3 * 0.7;
            i1 = i1 * 0.1;
        }
    }
    if (i3 > i1 && i3 > i2) {
        i3 = i3 * 0.8;
        if (i1 > i2) {
            i1 = i1 * 0.6;
            i2 = i2 * 0.3;
        } else {
            i1 = i1 * 0.3;
            i2 = i2 * 0.6;
        }
    }

    return sprintf("#%02x%02x%02x", i1, i2, i3);
}


export const getRandomMediumColor = (str) => {
    return getRandomColorWithRange(str, 96, 96);
}

const apiReq = (url, method, payload, type, resolve, reject, final) => {
    try {
        fetch(getApiUrlPrefix() + url, payload ? {method: method, body: JSON.stringify(payload)} : {method: method})
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
                        if (typeof data['result'] === 'object') {
                            if (data['last_modified_time']) {
                                data['result']['last_modified_time'] = DateTime.fromISO(data['last_modified_time']).setZone('local').toFormat('MM-dd HH:mm');
                            }
                            if (data['last_responded_time']) {
                                data['result']['last_responded_time'] = DateTime.fromISO(data['last_responded_time']).setZone('local').toFormat('MM-dd HH:mm');
                            }
                        }
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

export const jsonGetReq = (url, payload, resolve, reject, final) => apiReq(url, 'GET', payload, 'JSON', resolve, reject, final);
export const jsonPutReq = (url, payload, resolve, reject, final) => apiReq(url, 'PUT', payload, 'JSON', resolve, reject, final);
export const jsonDeleteReq = (url, payload, resolve, reject, final) => apiReq(url, 'DELETE', payload, 'JSON', resolve, reject, final);
export const textGetReq = (url, payload, resolve, reject, final) => apiReq(url, 'GET', payload, 'TEXT', resolve, reject, final);
export const blobGetReq = (url, payload, resolve, reject, final) => apiReq(url, 'GET', payload, 'BLOB', resolve, reject, final);

export const ROOT_DIRECTORY = '$$rootdir$$';
