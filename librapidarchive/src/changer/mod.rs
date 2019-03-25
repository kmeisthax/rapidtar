pub struct DataElement {
    pub volume_id: Option<String>,
    pub media_wear_indicator: Option<u64>
}

pub enum MediaElementStatus {
    Empty,
    Full(DataElement)
}

pub struct MediaSlot {
    pub fn index: u64;
    pub fn status: MediaElementStatus;
}

pub trait MediaChanger {
    pub fn get_slot_count() -> u64;

    pub fn get_slot(slot_id: u64) -> MediaSlot {
        
    }
}