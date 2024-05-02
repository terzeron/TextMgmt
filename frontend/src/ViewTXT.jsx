import {useEffect, useState, Suspense} from "react";
import PropTypes from 'prop-types';
import {textGetReq} from "./Common";

export default function ViewTXT(props) {
    const [errorMessage, setErrorMessage] = useState('');
    const [fileContent, setFileContent] = useState([]);

    useEffect(() => {
        if (props && props.bookId) {
            const downloadUrl = '/download/' + props.bookId;
            textGetReq(downloadUrl, null, (result) => {
                const lineList = result.split('\n').map(line => {
                        return line;
                    }
                );
                setFileContent(lineList);
            }, (error) => {
                setErrorMessage(`file content load failed, ${error}`);
            });
        }

        return () => {
            setFileContent('');
        };
    }, [props]);

    return (
        <div className="text-left">
            <Suspense fallback={<div className="loading">로딩 중...</div>}>
                {
                    errorMessage && <div>{errorMessage}</div>
                }
                {
                    fileContent &&
                    fileContent.map((line, index) => {
                        return <div key={index}>{line}</div>
                    })
                }
            </Suspense>
        </div>
    );
}

ViewTXT.propTypes = {
    bookId: PropTypes.number.isRequired,
    lineCount: PropTypes.number
}
