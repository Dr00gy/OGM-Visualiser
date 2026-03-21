# Bachelor's thesis
Optical Mapping Technology Sequence Mapping Visualization Tool / Nástroj pro vizualizaci mapování sekvencí technologie optického mapování

## Assignment
Optical genome mapping is a technology that allows for the investigation of large structural variants in the studied genome.
To observe deviations of the studied genome from the reference genome, it is necessary to map sequences, whether simulated or real, onto the reference sequence. The goal of this project should be to create an application that appropriately visualizes different sequence mappings onto the reference genome and enables visual identification of regions where accurate sequence mapping may be difficult. The visualization will focus on two views: the first view will examine one studied sequence and its alternative mappings across the genome, while the second view will focus on one genomic region, supplemented with various sequences. The guidelines for this project are as follows:

1) Familiarize yourself with optical mapping technology.
2) Design and implement optical mapping visualizations using the chosen technology.
3) The proposed visualization will allow for comparison of mappings to different reference sequences.

## Docker
After cloning the repo, just run `docker-compose up --build` !

## Testing
### Manual
.xmap files can be uploaded in the app by picking from PC or doing a drag and drop.

### Cargo test and Vitest
When testing the backend, run `cargo test` !

For the frontend tests, run `npm test` !
