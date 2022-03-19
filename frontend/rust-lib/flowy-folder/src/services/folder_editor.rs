use crate::services::web_socket::make_folder_ws_manager;
use flowy_sync::{
    client_folder::{FolderChange, FolderPad},
    entities::{revision::Revision, ws_data::ServerRevisionWSData},
};

use crate::manager::FolderId;
use bytes::Bytes;
use flowy_error::{FlowyError, FlowyResult};
use flowy_sync::util::make_delta_from_revisions;

use flowy_revision::{
    RevisionCloudService, RevisionCompactor, RevisionManager, RevisionObjectBuilder, RevisionWebSocket,
    RevisionWebSocketManager,
};
use lib_infra::future::FutureResult;
use lib_ot::core::PlainTextAttributes;

use parking_lot::RwLock;
use std::sync::Arc;

pub struct ClientFolderEditor {
    user_id: String,
    pub(crate) folder_id: FolderId,
    pub(crate) folder: Arc<RwLock<FolderPad>>,
    rev_manager: Arc<RevisionManager>,
    ws_manager: Arc<RevisionWebSocketManager>,
}

impl ClientFolderEditor {
    pub async fn new(
        user_id: &str,
        folder_id: &FolderId,
        token: &str,
        mut rev_manager: RevisionManager,
        web_socket: Arc<dyn RevisionWebSocket>,
    ) -> FlowyResult<Self> {
        let cloud = Arc::new(FolderRevisionCloudService {
            token: token.to_string(),
        });
        let folder = Arc::new(RwLock::new(rev_manager.load::<FolderPadBuilder>(Some(cloud)).await?));
        let rev_manager = Arc::new(rev_manager);
        let ws_manager = make_folder_ws_manager(
            user_id,
            folder_id.as_ref(),
            rev_manager.clone(),
            web_socket,
            folder.clone(),
        )
        .await;

        let user_id = user_id.to_owned();
        let folder_id = folder_id.to_owned();
        Ok(Self {
            user_id,
            folder_id,
            folder,
            rev_manager,
            ws_manager,
        })
    }

    pub async fn receive_ws_data(&self, data: ServerRevisionWSData) -> FlowyResult<()> {
        let _ = self.ws_manager.ws_passthrough_tx.send(data).await.map_err(|e| {
            let err_msg = format!("{} passthrough error: {}", self.folder_id, e);
            FlowyError::internal().context(err_msg)
        })?;
        Ok(())
    }

    pub(crate) fn apply_change(&self, change: FolderChange) -> FlowyResult<()> {
        let FolderChange { delta, md5 } = change;
        let (base_rev_id, rev_id) = self.rev_manager.next_rev_id_pair();
        let delta_data = delta.to_delta_bytes();
        let revision = Revision::new(
            &self.rev_manager.object_id,
            base_rev_id,
            rev_id,
            delta_data,
            &self.user_id,
            md5,
        );
        let _ = futures::executor::block_on(async {
            self.rev_manager
                .add_local_revision(&revision, Box::new(FolderRevisionCompactor()))
                .await
        })?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn folder_json(&self) -> FlowyResult<String> {
        let json = self.folder.read().to_json()?;
        Ok(json)
    }
}

struct FolderPadBuilder();
impl RevisionObjectBuilder for FolderPadBuilder {
    type Output = FolderPad;

    fn build_object(_object_id: &str, revisions: Vec<Revision>) -> FlowyResult<Self::Output> {
        let pad = FolderPad::from_revisions(revisions)?;
        Ok(pad)
    }
}

struct FolderRevisionCloudService {
    #[allow(dead_code)]
    token: String,
}

impl RevisionCloudService for FolderRevisionCloudService {
    #[tracing::instrument(level = "trace", skip(self))]
    fn fetch_object(&self, _user_id: &str, _object_id: &str) -> FutureResult<Vec<Revision>, FlowyError> {
        FutureResult::new(async move { Ok(vec![]) })
    }
}

#[cfg(feature = "flowy_unit_test")]
impl ClientFolderEditor {
    pub fn rev_manager(&self) -> Arc<RevisionManager> {
        self.rev_manager.clone()
    }
}

struct FolderRevisionCompactor();
impl RevisionCompactor for FolderRevisionCompactor {
    fn bytes_from_revisions(&self, revisions: Vec<Revision>) -> FlowyResult<Bytes> {
        let delta = make_delta_from_revisions::<PlainTextAttributes>(revisions)?;
        Ok(delta.to_delta_bytes())
    }
}
