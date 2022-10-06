import {useEffect, useRef, useState} from "react";
import {getUrlPrefix, handleFetchErrors} from "./Common";

export default function ViewTXT(props) {
  const [loading, setLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState('');
  const [fileContent, setFileContent] = useState('');

  useEffect(() => {
    console.log("222");
    if (props && props.entryId) {
      setLoading(true);
      const dirName = props.entryId.split('/')[0];
      const fileName = props.entryId.split('/')[1];
      let url = getUrlPrefix() + "/download/dirs/" + dirName + "/files/" + encodeURIComponent(fileName);
      if (props.size > 0) {
        url += "/size/" + props.size;
      }

      fetch(url)
        .then(handleFetchErrors)
        .then((response) => {
          response.json()
            .then((result) => {
              setFileContent(result);
              setErrorMessage(null);
            })
            .catch((error) => {
              setErrorMessage(error.message);
              setFileContent('');
            });
        })
        .catch((error) => {
          setErrorMessage(error.message);
          setFileContent('');
        })
        .finally(() => {
          setLoading(false);
        });
    }

    return () => {
      //console.log("cleanup");
    };
  }, [props]);

  if (loading) return <div>로딩 중...</div>;
  if (errorMessage) return <div>{errorMessage}</div>;
  return (
    <div className="text-left">
      {
        fileContent && fileContent.split('\n').map((line, index) => {
          return <div key={index}>{line}</div>
        })
      }
    </div>
  );
};


