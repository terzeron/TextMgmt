import {useEffect, useState, Suspense} from "react";
import {textGetReq} from "./Common";

export default function ViewTXT(props) {
  const [errorMessage, setErrorMessage] = useState('');
  const [fileContent, setFileContent] = useState('');

  useEffect(() => {
    console.log(`ViewTXT: useEffect(), props=`, props);
    if (props && props.entryId) {
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      const downloadUrl = '/download/dirs/' + dirName + '/files/' + encodeURIComponent(fileName);
      textGetReq(downloadUrl, (result) => {
        setFileContent(result);
      }, (error) => {
        setErrorMessage(`file content load failed, ${error}`);
      });
    }

    return () => {
      setFileContent('');
    };
  }, []);

  return (
    <div className="text-left">
      <Suspense fallback={<div className="loading">로딩 중...</div>}>
        {
          errorMessage && <div>{errorMessage}</div>
        }
        {
          fileContent && (
            props && props.lineCount > 0 ?
              fileContent.split('\n').slice(0, props.lineCount)
              :
              fileContent.split('\n')
          ).map((line, index) => {
            return <div key={index}>{line}</div>
          })
        }
      </Suspense>
    </div>
  );
}


