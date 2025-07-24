import {useEffect, Suspense, useState, useRef} from "react";
import PropTypes from 'prop-types';
import {RTFJS} from "rtf.js";
import {textGetReq} from "./Common";

export default function ViewRTF(props) {
    const [errorMessage, setErrorMessage] = useState('');
    const parentRef = useRef(null);

    const stringToArrayBuffer = (string) => {
        const buffer = new ArrayBuffer(string.length);
        const bufferView = new Uint8Array(buffer);
        for (let i = 0; i < string.length; i++) {
            bufferView[i] = string.charCodeAt(i);
        }
        return buffer;
    };

    useEffect(() => {
        if (props.bookId && props.filePath) {
            RTFJS.loggingEnabled(false);
            const downloadUrl = '/download/' + props.bookId + '/' + props.filePath;
            textGetReq(downloadUrl, null, (result) => {
                const doc = new RTFJS.Document(stringToArrayBuffer(result), null);
                doc.render().then((htmlElements) => {
                    const node = document.createElement("div");
                    htmlElements.map((element) => {
                        node.appendChild(element);
                    });
                    parentRef.current.appendChild(node);
                }).catch((error) => {
                    console.error(error)
                });
            }, (error) => {
                setErrorMessage(`file content load failed, ${error}`);
            });
        }
        return () => {
        };
    }, [props.bookId, props.filePath]);

    return (
        <div>
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                {
                    errorMessage && <div>{errorMessage}</div>
                }
                <div ref={parentRef}/>
            </Suspense>
        </div>
    );
}

ViewRTF.propTypes = {
    bookId: PropTypes.number.isRequired,
    filePath: PropTypes.string.isRequired
};