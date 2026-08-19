import os
from ament_index_python.packages import get_package_share_directory
from launch import LaunchDescription
from launch.actions import DeclareLaunchArgument, IncludeLaunchDescription
from launch.launch_description_sources import PythonLaunchDescriptionSource
from launch.substitutions import LaunchConfiguration
from launch_ros.actions import Node

# Sanitize Snap library contamination from LD_LIBRARY_PATH if running from Snap VS Code / terminal
if 'LD_LIBRARY_PATH' in os.environ:
    os.environ['LD_LIBRARY_PATH'] = ':'.join([
        p for p in os.environ['LD_LIBRARY_PATH'].split(':')
        if '/snap/' not in p
    ])

def generate_launch_description():
    pkg_mio_description = get_package_share_directory('mio_description')
    rsp_launch_path = os.path.join(pkg_mio_description, 'launch', 'rsp.launch.py')
    default_rviz_config = os.path.join(pkg_mio_description, 'config', 'view_robot.rviz')

    rviz_config_arg = DeclareLaunchArgument(
        'rviz_config',
        default_value=default_rviz_config,
        description='Absolute path to rviz config file'
    )

    rviz_config = LaunchConfiguration('rviz_config')

    # Include Robot State Publisher
    rsp_launch = IncludeLaunchDescription(
        PythonLaunchDescriptionSource(rsp_launch_path),
        launch_arguments={'use_sim_time': 'false'}.items()
    )

    # Joint State Publisher GUI
    node_joint_state_publisher_gui = Node(
        package='joint_state_publisher_gui',
        executable='joint_state_publisher_gui',
        output='screen'
    )

    # RViz2
    node_rviz = Node(
        package='rviz2',
        executable='rviz2',
        arguments=['-d', rviz_config],
        output='screen'
    )

    return LaunchDescription([
        rviz_config_arg,
        rsp_launch,
        node_joint_state_publisher_gui,
        node_rviz,
    ])
