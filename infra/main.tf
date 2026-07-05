locals {
  common_tags = {
    project = var.project_name
  }

  base_domain_regex = replace(var.base_domain, ".", "\\.")
}

data "oci_identity_availability_domains" "available" {
  compartment_id = var.tenancy_ocid
}

resource "oci_core_vcn" "main" {
  compartment_id = var.compartment_ocid
  cidr_block     = var.vcn_cidr
  display_name   = "${var.project_name}-vcn"
  dns_label      = var.vcn_dns_label
  freeform_tags  = local.common_tags
}

resource "oci_core_internet_gateway" "main" {
  compartment_id = var.compartment_ocid
  display_name   = "${var.project_name}-internet-gateway"
  enabled        = true
  vcn_id         = oci_core_vcn.main.id
  freeform_tags  = local.common_tags
}

resource "oci_core_route_table" "public" {
  compartment_id = var.compartment_ocid
  display_name   = "${var.project_name}-public-routes"
  vcn_id         = oci_core_vcn.main.id
  freeform_tags  = local.common_tags

  route_rules {
    destination       = "0.0.0.0/0"
    destination_type  = "CIDR_BLOCK"
    network_entity_id = oci_core_internet_gateway.main.id
  }
}

resource "oci_core_security_list" "public" {
  compartment_id = var.compartment_ocid
  display_name   = "${var.project_name}-public-security"
  vcn_id         = oci_core_vcn.main.id
  freeform_tags  = local.common_tags

  dynamic "ingress_security_rules" {
    for_each = var.allowed_ssh_cidrs

    content {
      protocol = "6"
      source   = ingress_security_rules.value

      tcp_options {
        min = 22
        max = 22
      }
    }
  }

  ingress_security_rules {
    protocol = "6"
    source   = "0.0.0.0/0"

    tcp_options {
      min = 80
      max = 80
    }
  }

  ingress_security_rules {
    protocol = "6"
    source   = "0.0.0.0/0"

    tcp_options {
      min = 443
      max = 443
    }
  }

  egress_security_rules {
    destination = "0.0.0.0/0"
    protocol    = "all"
  }
}

resource "oci_core_subnet" "public" {
  cidr_block                 = var.public_subnet_cidr
  compartment_id             = var.compartment_ocid
  display_name               = "${var.project_name}-public-subnet"
  dns_label                  = var.subnet_dns_label
  prohibit_public_ip_on_vnic = false
  route_table_id             = oci_core_route_table.public.id
  security_list_ids          = [oci_core_security_list.public.id]
  vcn_id                     = oci_core_vcn.main.id
  freeform_tags              = local.common_tags
}

resource "oci_core_instance" "app" {
  availability_domain = data.oci_identity_availability_domains.available.availability_domains[var.availability_domain_index].name
  compartment_id      = var.compartment_ocid
  display_name        = "${var.project_name}-arm-vm"
  shape               = "VM.Standard.A1.Flex"
  freeform_tags       = local.common_tags

  shape_config {
    memory_in_gbs = var.instance_memory_gb
    ocpus         = var.instance_ocpus
  }

  create_vnic_details {
    assign_public_ip = true
    display_name     = "${var.project_name}-public-vnic"
    hostname_label   = var.instance_hostname_label
    subnet_id        = oci_core_subnet.public.id
  }

  source_details {
    boot_volume_size_in_gbs = var.boot_volume_size_gb
    source_id               = var.instance_image_ocid
    source_type             = "image"
  }

  metadata = {
    ssh_authorized_keys = file(pathexpand(var.ssh_public_key_path))
    user_data = base64encode(templatefile("${path.module}/cloud-init.yaml.tftpl", {
      base_domain               = var.base_domain
      base_domain_regex         = local.base_domain_regex
      project_name              = var.project_name
      repository_url            = var.repository_url
      sqlite_schema_b64         = base64encode(file("${path.module}/sqlite-schema.sql"))
      support_email             = var.support_email
      support_telegram          = var.support_telegram
      support_tickets           = var.support_tickets
      zola_version              = var.zola_version
      install_rustup_on_boot    = var.install_rustup_on_boot
      install_zola_on_boot      = var.install_zola_on_boot
      install_evernote_gui_deps = var.install_evernote_gui_deps
      install_evernote_appimage = var.install_evernote_appimage_on_boot
      run_evernote_on_boot      = var.run_evernote_on_boot
      evernote_appimage_repo    = var.evernote_appimage_repository
      evernote_appimage_regex   = var.evernote_appimage_asset_regex
    }))
  }
}
