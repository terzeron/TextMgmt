import {useEffect, Suspense, useState, useRef} from "react";
import {RTFJS} from "rtf.js";
import {textGetReq} from "./Common";

export default function ViewRTF(props) {
  const [errorMessage, setErrorMessage] = useState('');
  const parentRef = useRef("parentRef");

  const rtf =
    `{\\rtf1\\ansi\\ansicpg1252\\deff0\\deflang1033{\\fonttbl{\\f0\\fnil\\fcharset0 Calibri;}}
    {\\*\\generator Msftedit 5.41.21.2510;}\\viewkind4\\uc1\\pard\\sa200\\sl276\\slmult1\\lang9\\f0\\fs22 This \\fs44 is \\fs22 a \\b simple \\ul one \\i paragraph \\ulnone\\b0 document\\i0 .\\par
    }`;
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
        const doc = new RTFJS.Document(stringToArrayBuffer(result));
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