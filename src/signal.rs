use serde::{Deserialize, Serialize};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

/// Messages sent from browser → server
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMsg {
    Offer  { sdp: String },
    Answer { sdp: String },
    #[serde(rename = "ice-candidate")]
    IceCandidate { candidate: IceCandidateInit },
}

/// Messages sent from server → browser
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMsg {
    #[serde(rename = "you-are")]
    YouAre  { user_id: String },
    Answer  { sdp: String },
    Offer   { sdp: String },
    #[serde(rename = "user-left")]
    UserLeft { user_id: String },
    #[serde(rename = "ice-candidate")]
    IceCandidate { candidate: IceCandidateInit },
}

/// ICE candidate with browser-compatible field names
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidateInit {
    pub candidate: String,
    #[serde(rename = "sdpMid")]
    pub sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    pub sdp_mline_index: Option<u16>,
    #[serde(rename = "usernameFragment", skip_serializing_if = "Option::is_none")]
    pub username_fragment: Option<String>,
}

impl From<IceCandidateInit> for RTCIceCandidateInit {
    fn from(c: IceCandidateInit) -> Self {
        RTCIceCandidateInit {
            candidate:          c.candidate,
            sdp_mid:            c.sdp_mid,
            sdp_mline_index:    c.sdp_mline_index,
            username_fragment:  c.username_fragment,
        }
    }
}

impl From<RTCIceCandidateInit> for IceCandidateInit {
    fn from(c: RTCIceCandidateInit) -> Self {
        IceCandidateInit {
            candidate:          c.candidate,
            sdp_mid:            c.sdp_mid,
            sdp_mline_index:    c.sdp_mline_index,
            username_fragment:  c.username_fragment,
        }
    }
}
