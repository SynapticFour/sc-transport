resource "oci_core_instance" "sct_sender" {
  availability_domain = data.oci_identity_availability_domain.sender_ad.name
  compartment_id      = var.compartment_ocid
  display_name        = "sct-wan-sender"
  shape               = "VM.Standard.A1.Flex"

  shape_config {
    ocpus         = 2
    memory_in_gbs = 12
  }

  source_details {
    source_type = "image"
    source_id   = data.oci_core_images.ubuntu_arm.images[0].id
  }

  create_vnic_details {
    subnet_id        = oci_core_subnet.sender_subnet.id
    assign_public_ip = true
    display_name     = "sct-wan-sender-vnic"
  }

  metadata = {
    ssh_authorized_keys = var.ssh_public_key
    user_data = base64encode(templatefile("${path.module}/cloud-init-sender.sh", {
      github_repo = var.github_repo
      github_ref  = var.github_ref
      sct_port    = var.sct_port
    }))
  }

  freeform_tags = {
    "project" = "sc-transport"
    "role"    = "wan-sender"
  }
}

resource "oci_core_instance" "sct_receiver" {
  provider            = oci.receiver
  availability_domain = data.oci_identity_availability_domain.receiver_ad.name
  compartment_id      = var.compartment_ocid
  display_name        = "sct-wan-receiver"
  shape               = "VM.Standard.A1.Flex"

  shape_config {
    ocpus         = 2
    memory_in_gbs = 12
  }

  source_details {
    source_type = "image"
    source_id   = data.oci_core_images.ubuntu_arm_receiver.images[0].id
  }

  create_vnic_details {
    subnet_id        = oci_core_subnet.receiver_subnet.id
    assign_public_ip = true
    display_name     = "sct-wan-receiver-vnic"
  }

  metadata = {
    ssh_authorized_keys = var.ssh_public_key
    user_data = base64encode(templatefile("${path.module}/cloud-init-receiver.sh", {
      github_repo = var.github_repo
      github_ref  = var.github_ref
      sct_port    = var.sct_port
    }))
  }

  freeform_tags = {
    "project" = "sc-transport"
    "role"    = "wan-receiver"
  }
}

data "oci_core_vnic_attachments" "sender_vnics" {
  compartment_id = var.compartment_ocid
  instance_id    = oci_core_instance.sct_sender.id
}

data "oci_core_vnic" "sender_vnic" {
  vnic_id = data.oci_core_vnic_attachments.sender_vnics.vnic_attachments[0].vnic_id
}

data "oci_core_vnic_attachments" "receiver_vnics" {
  provider       = oci.receiver
  compartment_id = var.compartment_ocid
  instance_id    = oci_core_instance.sct_receiver.id
}

data "oci_core_vnic" "receiver_vnic" {
  provider = oci.receiver
  vnic_id  = data.oci_core_vnic_attachments.receiver_vnics.vnic_attachments[0].vnic_id
}
