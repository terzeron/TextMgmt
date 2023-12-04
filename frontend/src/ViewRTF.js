import {useEffect, Suspense, useState, useRef} from "react";
import {RTFJS} from "rtf.js";
import {textGetReq} from "./Common";

export default function ViewRTF(props) {
  const [errorMessage, setErrorMessage] = useState('');
  const parentRef = useRef("parentRef");

  const stringToArrayBuffer = (string) => {
    const buffer = new ArrayBuffer(string.length);
    const bufferView = new Uint8Array(buffer);
    for (let i = 0; i < string.length; i++) {
      bufferView[i] = string.charCodeAt(i);
    }
    return buffer;
  };

  useEffect(() => {
    console.log(`ViewRTF: useEffect()`, props);
    RTFJS.loggingEnabled(false);
    if (props && props.entryId) {
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const downloadUrl = '/download/dirs/' + dirName + '/files/' + encodeURIComponent(fileName);
      textGetReq(downloadUrl, (result) => {
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
  }, []);

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