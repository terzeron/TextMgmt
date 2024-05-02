import {useEffect, useState} from "react";
import PropTypes from 'prop-types';

import './Edit.css';
import 'bootstrap/dist/css/bootstrap.min.css';

import {Button, Form, InputGroup, Row} from 'react-bootstrap';
import {FontAwesomeIcon} from '@fortawesome/react-fontawesome';
import {faTruckMoving, faUpload} from '@fortawesome/free-solid-svg-icons';

import {getRandomMediumColor, ROOT_DIRECTORY} from './Common';


export default function Move(props) {
    const [renderingInfoList, setRenderingInfoList] = useState([]);

    useEffect(() => {
        const infoList = props.otherCategoryList?.map(category => {
            const hasSubCategory = category.includes('_');
            if (hasSubCategory) {
                const subCategory = category.split('_')[1];
                return {key: category, label: subCategory, style: {backgroundColor: getRandomMediumColor(category), color: 'white'}};
            }
            return {key: category, label: category, style: {}, class: 'btn-light'};
        });
        setRenderingInfoList(infoList);
    }, [props]);

    return (
        <>
            <Row className="button_group">
                {
                    !props.selectedEntryId.startsWith(ROOT_DIRECTORY) > 0 &&
                    <Button variant="outline-warning" size="sm" onClick={props.moveToUpperButtonClicked} disabled={!props.newFileName}>
                        상위로
                        <FontAwesomeIcon icon={faUpload}/>
                    </Button>
                }
                {
                    renderingInfoList.map(info =>
                        (
                            <Button
                                variant="outline-secondary"
                                size="sm"
                                key={info['key']}
                                className={info['class']}
                                style={info['style']}
                                onClick={(e) => {
                                    props.selectDirectoryButtonClicked(e, info['key']);
                                }}>
                                {info['label']}
                            </Button>
                        )
                    )
                }
            </Row>

            <Row>
                <InputGroup className="ms-0 me-0">
                    <Form.Control value={props.selectedCategory} readOnly/>
                    <Button variant="outline-warning" size="sm" onClick={props.moveToDirectoryButtonClicked
                    } disabled={!props.selectedEntryId && !props.selectedCategory}>
                        로 옮기기
                        <FontAwesomeIcon icon={faTruckMoving}/>
                    </Button>
                </InputGroup>
            </Row>
        </>
    )
        ;
}

Move.propTypes = {
    selectedEntryId: PropTypes.string.isRequired,
    selectedCategory: PropTypes.string.isRequired,
    otherCategoryList: PropTypes.array.isRequired,
    moveToUpperButtonClicked: PropTypes.func,
    moveToDirectoryButtonClicked: PropTypes.func,
    selectDirectoryButtonClicked: PropTypes.func,
    newFileName: PropTypes.string.isRequired,
};
